import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@tauri-apps/api/core";
import * as api from "./api";
import type { AppSettings, UsageFilters } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * The panel reaches the Core two ways — Tauri IPC in the desktop shell, HTTP on
 * loopback in a browser — and both must ask for exactly the same thing. These
 * tests pin that contract from the client side: which command name and which
 * URL each call maps to, and that a failure surfaces as an error rather than a
 * silently empty result.
 */
const TAURI_MARKER = "__TAURI_INTERNALS__";

function jsonResponse(body: unknown, ok = true, status = 200): Response {
  return {
    ok,
    status,
    json: async () => body,
  } as Response;
}

// The transport is chosen by sniffing this marker on `window`, so the tests
// set and clear it the same way the Tauri runtime does.
function taurified(): Record<string, unknown> {
  return window as unknown as Record<string, unknown>;
}

function useBrowserRuntime() {
  delete taurified()[TAURI_MARKER];
}

function useDesktopRuntime() {
  taurified()[TAURI_MARKER] = {};
}

describe("api transport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useBrowserRuntime();
  });

  afterEach(() => {
    useBrowserRuntime();
    vi.unstubAllGlobals();
  });

  it("reports which runtime it is in", () => {
    expect(api.isDesktopRuntime()).toBe(false);
    useDesktopRuntime();
    expect(api.isDesktopRuntime()).toBe(true);
  });

  it("reads through loopback HTTP in a browser", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    await api.getQuickSummary();

    expect(fetchMock).toHaveBeenCalledWith("/api/quick-summary", undefined);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reads through Tauri IPC in the desktop shell", async () => {
    useDesktopRuntime();
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    vi.mocked(invoke).mockResolvedValue({ ok: true });

    await api.getQuickSummary();

    expect(invoke).toHaveBeenCalledWith("get_quick_summary", undefined);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("turns a failed response into an error instead of returning its body", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(jsonResponse({ error: "核心不可用" }, false, 500)),
    );

    await expect(api.listSources()).rejects.toThrow("核心不可用");
  });

  it("falls back to the status code when the failure carries no message", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(jsonResponse({}, false, 503)),
    );

    await expect(api.listProviders()).rejects.toThrow("请求失败（503）");
  });

  it("omits empty filter values from the query string", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse([]));
    vi.stubGlobal("fetch", fetchMock);

    const filters: UsageFilters = {
      period_start: "2026-07-26T00:00:00Z",
      // Empty and null values must not become `app=` or `model=null`, which the
      // server would read as a filter for the empty string.
      app: null,
      model: "",
      search: "codex",
    };
    await api.listSessions(filters, 25, 50);

    const [url] = fetchMock.mock.calls[0] as [string];
    expect(url).toContain("/api/sessions?");
    expect(url).toContain("period_start=2026-07-26T00%3A00%3A00Z");
    expect(url).toContain("search=codex");
    expect(url).toContain("limit=25");
    expect(url).toContain("offset=50");
    expect(url).not.toContain("app=");
    expect(url).not.toContain("model=");
  });

  it("uses no query string when nothing is filtered", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse([]));
    vi.stubGlobal("fetch", fetchMock);

    await api.listQuotaSnapshots(null, 100);
    const [url] = fetchMock.mock.calls[0] as [string];
    expect(url).toBe("/api/quotas?limit=100");

    await api.listAccounts();
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/accounts");
  });

  it("encodes a session id into its path", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(null));
    vi.stubGlobal("fetch", fetchMock);

    await api.getSessionDetail("codex-session:a/b c");

    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/sessions/codex-session%3Aa%2Fb%20c",
    );
  });

  it("writes settings with PUT and a JSON body in the browser", async () => {
    const settings: AppSettings = {
      codex_home: "/sanitized/codex",
      claude_home: null,
      cc_switch_db_path: null,
      cockpit_path: null,
      otel_port: null,
      auto_start: true,
      proxy_enabled: false,
      save_request_metadata: false,
      data_retention_days: 30,
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(settings));
    vi.stubGlobal("fetch", fetchMock);

    await api.updateAppSettings(settings);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/settings");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(String(init.body))).toEqual(settings);
  });

  it("writes settings over IPC without an HTTP body in the desktop shell", async () => {
    useDesktopRuntime();
    vi.mocked(invoke).mockResolvedValue({});
    const settings = { auto_start: false } as AppSettings;

    await api.updateAppSettings(settings);

    expect(invoke).toHaveBeenCalledWith("update_app_settings", { settings });
  });

  it("posts rescan requests with the snake_case field the server expects", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({}));
    vi.stubGlobal("fetch", fetchMock);

    await api.rescanCodex("/sanitized/codex");
    await api.rescanClaude(null);
    await api.rescanCcSwitch("/sanitized/cc.db");
    await api.rescanCockpit("/sanitized/cockpit.sqlite");

    const bodies = fetchMock.mock.calls.map(([, init]) =>
      JSON.parse(String((init as RequestInit).body)),
    );
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/rescan-codex",
      "/api/rescan-claude",
      "/api/rescan-cc-switch",
      "/api/rescan-cockpit",
    ]);
    expect(bodies).toEqual([
      { codex_home: "/sanitized/codex" },
      { claude_home: null },
      { cc_switch_db: "/sanitized/cc.db" },
      { cockpit_db: "/sanitized/cockpit.sqlite" },
    ]);
  });

  it("passes detection paths as query parameters", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({}));
    vi.stubGlobal("fetch", fetchMock);

    await api.detectCodexPath("/sanitized/codex");
    await api.detectClaudePath(null);
    await api.detectCcSwitchPath("/sanitized/cc.db");
    await api.detectCockpitPath(null);

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/detect-codex?codex_home=%2Fsanitized%2Fcodex",
      "/api/detect-claude",
      "/api/detect-cc-switch?cc_switch_db=%2Fsanitized%2Fcc.db",
      "/api/detect-cockpit",
    ]);
  });

  it("posts an export request and returns the rendered content", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse({
        filename: "usage.csv",
        mime_type: "text/csv",
        content: "id\n",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await api.exportUsage("csv", { app: "codex" });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/export");
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({
      format: "csv",
      filters: { app: "codex" },
    });
    expect(result.filename).toBe("usage.csv");
  });

  it("routes the remaining read endpoints to their loopback paths", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({}));
    vi.stubGlobal("fetch", fetchMock);

    await api.getDashboardSummary({});
    await api.getModelBreakdown({});
    await api.listUsageEvents(null, 10, 0);
    await api.listUsageEvents("session-1", 10, 0);

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/dashboard-summary",
      "/api/model-breakdown",
      // A zero offset is sent explicitly; only null and empty values are dropped.
      "/api/usage-events?limit=10&offset=0",
      "/api/usage-events?session_id=session-1&limit=10&offset=0",
    ]);
  });

  it("answers the local-web-api questions locally when already in a browser", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    // The browser panel *is* the local web API, so asking it to start or report
    // itself is answered without a round trip.
    for (const status of [
      await api.getLocalWebApiStatus(),
      await api.startLocalWebApi(),
      await api.openLocalWebApi(),
    ]) {
      expect(status.running).toBe(true);
      expect(status.url).toBe(window.location.origin);
    }
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("exposes the desktop-only commands as direct IPC calls", async () => {
    useDesktopRuntime();
    vi.mocked(invoke).mockResolvedValue(null);

    await api.pickDirectory("选择 Codex Home", "/sanitized/codex");
    await api.pickFile("选择数据库", null);
    await api.saveExport("json", {});
    await api.showMainWindow();
    await api.startLocalWebApi();
    await api.openLocalWebApi();
    await api.getLocalWebApiStatus();

    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual([
      "pick_directory",
      "pick_file",
      "save_export",
      "show_main_window",
      "start_local_web_api",
      "open_local_web_api",
      "get_local_web_api_status",
    ]);
  });
});
