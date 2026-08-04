import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import App from "./App";
import {
  detectClaudePath,
  detectCodexPath,
  exportUsage,
  fitQuickWindowToContent,
  getAppSettings,
  getDashboardSummary,
  getModelBreakdown,
  getQuickSummary,
  getSessionDetail,
  isDesktopRuntime,
  listAccounts,
  listProviders,
  listQuotaSnapshots,
  listSessions,
  listSources,
  openLocalWebApi,
  pickDirectory,
  refreshOfficialQuota,
  rescanCcSwitch,
  rescanClaude,
  rescanCockpit,
  rescanCodex,
  saveExport,
  showMainWindow,
  updateAppSettings,
} from "./lib/api";

vi.mock("./lib/api", () => ({
  detectClaudePath: vi.fn(),
  detectCodexPath: vi.fn(),
  detectCcSwitchPath: vi.fn(),
  detectCockpitPath: vi.fn(),
  getAppSettings: vi.fn(),
  getDashboardSummary: vi.fn(),
  getModelBreakdown: vi.fn(),
  getQuickSummary: vi.fn(),
  getSessionDetail: vi.fn(),
  exportUsage: vi.fn(),
  fitQuickWindowToContent: vi.fn(() => Promise.resolve()),
  listAccounts: vi.fn(),
  listProviders: vi.fn(),
  listQuotaSnapshots: vi.fn(),
  listSessions: vi.fn(),
  listSources: vi.fn(),
  openLocalWebApi: vi.fn(),
  pickDirectory: vi.fn(),
  pickFile: vi.fn(),
  refreshOfficialQuota: vi.fn(),
  rescanClaude: vi.fn(),
  rescanCodex: vi.fn(),
  rescanCcSwitch: vi.fn(),
  rescanCockpit: vi.fn(),
  saveExport: vi.fn(),
  showMainWindow: vi.fn(),
  isDesktopRuntime: vi.fn(() => false),
  updateAppSettings: vi.fn(),
}));

const totals = {
  event_count: 2,
  input_tokens_total: 100,
  input_tokens_uncached: 75,
  cache_read_tokens: 25,
  cache_write_tokens: null,
  output_tokens_total: 30,
  reasoning_tokens: 5,
  visible_output_tokens: 25,
  provider_reported_cost: null,
  estimated_cost: null,
  cache_hit_rate_percent: 25,
};

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getDashboardSummary).mockResolvedValue({
      period_start: "2026-07-26T00:00:00Z",
      period_end: "2026-07-27T00:00:00Z",
      totals,
    });
    vi.mocked(getModelBreakdown).mockResolvedValue([]);
    vi.mocked(listSessions).mockResolvedValue({ sessions: [], total: 0 });
    vi.mocked(listSources).mockResolvedValue([]);
    vi.mocked(listProviders).mockResolvedValue([]);
    vi.mocked(listQuotaSnapshots).mockResolvedValue([]);
    vi.mocked(listAccounts).mockResolvedValue([]);
    vi.mocked(getAppSettings).mockResolvedValue({
      codex_home: null,
      claude_home: null,
      cc_switch_db_path: null,
      cockpit_path: null,
      otel_port: null,
      auto_start: false,
      proxy_enabled: false,
      save_request_metadata: false,
      data_retention_days: null,
    });
    vi.mocked(updateAppSettings).mockImplementation(
      async (settings) => settings,
    );
    vi.mocked(getSessionDetail).mockResolvedValue(null);
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_id: "session-1",
      active_session_title: "Fixture session",
      active_project_path: "/sanitized/project",
      provider_name: "OpenAI",
      model: "gpt-5-codex",
      session_input_tokens: 100,
      session_cache_read_tokens: 20,
      session_output_tokens: 40,
      session_cache_hit_rate: 20,
      today_total_tokens: 140,
      quota_summary: null,
      latest_warning: null,
    });
  });

  afterEach(() => {
    window.history.pushState({}, "", "/");
  });

  it("renders the dashboard shell and loads local totals", async () => {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "TokenBuddy" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "扫描全部来源" }),
    ).toBeInTheDocument();
    expect(screen.getByText("输入 Token")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("数据已从本地 SQLite 加载")).toBeInTheDocument();
    });
    expect(screen.getByText("100")).toBeInTheDocument();
    expect(screen.getByText("25.0%")).toBeInTheDocument();
  });

  it("breaks usage down by model and the provider that actually served it", async () => {
    vi.mocked(getModelBreakdown).mockResolvedValue([
      {
        model: "deepseek-v4-pro",
        provider_id: "cc-switch:claude:deepseek",
        // Attributed by the launcher, not guessed from the model name.
        provider_name: "DeepSeek",
        app: "claude_code",
        totals,
      },
    ]);
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("deepseek-v4-pro")).toBeInTheDocument();
    });
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "按模型 / 供应商" }),
    ).toBeInTheDocument();
  });

  it("renders the tray quick view from the Core-owned QuickSummary boundary", async () => {
    window.history.pushState({}, "", "/quick");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("今日 Token")).toBeInTheDocument();
    });
    expect(screen.getByText("140")).toBeInTheDocument();
    expect(screen.getByText("采集中")).toBeInTheDocument();
    expect(screen.getByText("Fixture session")).toBeInTheDocument();
    expect(screen.getByText("项目：/sanitized/project")).toBeInTheDocument();
    expect(screen.getByText(/OpenAI/)).toBeInTheDocument();
    expect(getQuickSummary).toHaveBeenCalled();
    expect(listSessions).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("combobox", { name: "选择会话" }),
    ).not.toBeInTheDocument();
  });

  it("uses the active session totals returned by QuickSummary", async () => {
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_id: "session-1",
      active_session_title: "真实会话标题",
      active_project_path: "/sanitized/project",
      provider_name: "OpenAI",
      model: "gpt-5-codex",
      session_input_tokens: 220,
      session_cache_read_tokens: 80,
      session_output_tokens: 70,
      session_cache_hit_rate: 36.4,
      today_total_tokens: 290,
      quota_summary: null,
      latest_warning: null,
    });
    window.history.pushState({}, "", "/quick");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("真实会话标题")).toBeInTheDocument();
    });
    expect(screen.getByText("220")).toBeInTheDocument();
    expect(screen.getByText("70")).toBeInTheDocument();
  });

  it("exposes desktop-only tray actions that open the full dashboard", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(true);
    vi.mocked(showMainWindow).mockResolvedValue();
    window.history.pushState({}, "", "/quick");
    render(<App />);

    const openButton = await screen.findByRole("button", {
      name: "打开完整面板…",
    });
    fireEvent.click(openButton);
    expect(showMainWindow).toHaveBeenCalled();
  });

  it.each([
    ["/providers", "Providers"],
    ["/quotas", "官方额度"],
    ["/settings", "设置"],
  ])("renders the shared SPA route %s", async (path, heading) => {
    window.history.pushState({}, "", path);
    render(<App />);

    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: heading }),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByRole("navigation", { name: "主要导航" }),
    ).toBeInTheDocument();
  });

  it("keeps the empty provider and quota states explicit", async () => {
    window.history.pushState({}, "", "/providers");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Provider 数据 Unavailable")).toBeInTheDocument();
    });
    expect(listProviders).toHaveBeenCalled();

    window.history.pushState({}, "", "/quotas");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("官方额度 Unavailable")).toBeInTheDocument();
    });
    expect(screen.getByText("账号 Unavailable")).toBeInTheDocument();
    expect(listQuotaSnapshots).toHaveBeenCalled();
    expect(listAccounts).toHaveBeenCalled();
  });

  it("shows the official Codex account with its plan and quota window", async () => {
    vi.mocked(listAccounts).mockResolvedValue([
      {
        account: {
          id: "openai:chatgpt:fixture00000000",
          provider_id: "openai",
          display_name: "fixture@example.com",
          account_fingerprint: "fixture00000000feedfacefeedface",
          auth_mode: "chatgpt",
          plan: "pro",
        },
        provider_name: "OpenAI",
        latest_quota: {
          window_type: "primary_5h",
          used_percent: 18.75,
          remaining_percent: 81.25,
          reset_at: "2026-07-26T11:00:00Z",
          credits_remaining: null,
          precision: "correlated",
        },
      },
    ]);
    window.history.pushState({}, "", "/quotas");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("fixture@example.com")).toBeInTheDocument();
    });
    expect(screen.getByText("OpenAI · ChatGPT 官方登录")).toBeInTheDocument();
    expect(screen.getByText("pro")).toBeInTheDocument();
    // The window is shown at the precision it was recorded with, never as
    // Verified, and the fingerprint is truncated.
    expect(screen.getByText("primary_5h 18.8% 已用")).toBeInTheDocument();
    expect(screen.getByText("Correlated")).toBeInTheDocument();
    expect(screen.getByText("指纹 fixture00000")).toBeInTheDocument();
  });

  it("fills a settings path from the native picker without saving it", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(true);
    vi.mocked(pickDirectory).mockResolvedValue("/picked/codex");
    window.history.pushState({}, "", "/settings");
    render(<App />);

    const browse = await screen.findByRole("button", {
      name: "Codex Home：浏览",
    });
    fireEvent.click(browse);

    await waitFor(() => {
      expect(screen.getByLabelText("Codex Home")).toHaveValue("/picked/codex");
    });
    expect(pickDirectory).toHaveBeenCalledWith("选择 Codex Home", null);
    // Picking a path must not write it: saving stays an explicit action.
    expect(updateAppSettings).not.toHaveBeenCalled();
  });

  it("keeps the picker out of the browser panel", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(false);
    window.history.pushState({}, "", "/settings");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByLabelText("Codex Home")).toBeInTheDocument();
    });
    expect(
      screen.queryByRole("button", { name: "Codex Home：浏览" }),
    ).not.toBeInTheDocument();
  });
});

/**
 * The panels' behaviour beyond first render: the actions a user actually
 * clicks, and what each view shows when its query fails. A failed query must
 * become a visible message — never an empty panel that reads as "no usage".
 */
describe("App panels", () => {
  const session = {
    session: {
      id: "codex-session:abc",
      external_session_id: "abc",
      parent_session_id: null,
      app: "codex" as const,
      launcher: "direct" as const,
      project_path: "/sanitized/project",
      title: "Fixture session",
      started_at: "2026-07-26T08:00:00Z",
      ended_at: "2026-07-26T08:10:00Z",
      source_id: "codex-session",
      created_at: "2026-07-26T08:00:00Z",
      updated_at: "2026-07-26T08:10:00Z",
    },
    totals,
  };

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getDashboardSummary).mockResolvedValue({
      period_start: "2026-07-26T00:00:00Z",
      period_end: "2026-07-27T00:00:00Z",
      totals,
    });
    vi.mocked(getModelBreakdown).mockResolvedValue([]);
    vi.mocked(listSessions).mockResolvedValue({ sessions: [], total: 0 });
    vi.mocked(listSources).mockResolvedValue([]);
    vi.mocked(listProviders).mockResolvedValue([]);
    vi.mocked(listQuotaSnapshots).mockResolvedValue([]);
    vi.mocked(listAccounts).mockResolvedValue([]);
    vi.mocked(getSessionDetail).mockResolvedValue(null);
    vi.mocked(getAppSettings).mockResolvedValue({
      codex_home: null,
      claude_home: null,
      cc_switch_db_path: null,
      cockpit_path: null,
      otel_port: null,
      auto_start: false,
      proxy_enabled: false,
      save_request_metadata: false,
      data_retention_days: null,
    });
  });

  afterEach(() => {
    window.history.pushState({}, "", "/");
  });

  it("lists sessions and reports a failure instead of showing an empty list", async () => {
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session],
      total: 1,
    });
    window.history.pushState({}, "", "/sessions");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Fixture session")).toBeInTheDocument();
    });
    // App and project share one line in the session row.
    expect(screen.getByText(/\/sanitized\/project/)).toBeInTheDocument();

    vi.mocked(listSessions).mockRejectedValue(new Error("核心不可用"));
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/无法读取会话列表/)).toBeInTheDocument();
    });
  });

  it("shows a session's request timeline, and says so when the id is unknown", async () => {
    vi.mocked(getSessionDetail).mockResolvedValue({
      summary: session,
      usage_events: [
        {
          id: "event-1",
          occurred_at: "2026-07-26T08:00:01Z",
          app: "codex",
          launcher: "direct",
          ingest_source: "session_log",
          source_id: "codex-session",
          provider_id: "openai",
          account_id: null,
          session_id: "codex-session:abc",
          parent_session_id: null,
          request_id: "request-001",
          response_id: null,
          model: "gpt-5-codex",
          query_source: null,
          usage: {
            input_tokens_total: 100,
            input_tokens_uncached: 75,
            cache_read_tokens: 25,
            cache_write_tokens: null,
            output_tokens_total: 30,
            reasoning_tokens: 5,
            visible_output_tokens: 25,
          },
          provider_reported_cost: null,
          estimated_cost: null,
          currency: null,
          http_status: null,
          latency_ms: null,
          success: true,
          precision_token: "exact_session",
          precision_session: "exact_session",
          precision_provider: "unavailable",
          precision_account: "unavailable",
          raw_event_hash: "event-1",
          raw_usage_json: null,
        },
      ],
    });
    window.history.pushState({}, "", "/sessions/codex-session%3Aabc");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText(/request-001/)).toBeInTheDocument();
    });
    expect(screen.getAllByText("Exact session").length).toBeGreaterThan(0);
    expect(getSessionDetail).toHaveBeenCalledWith("codex-session:abc");

    vi.mocked(getSessionDetail).mockResolvedValue(null);
    render(<App />);
    await waitFor(() => {
      expect(
        screen.getAllByText(/会话不存在|Unavailable/).length,
      ).toBeGreaterThan(0);
    });
  });

  it("shows each source's health on the sources page", async () => {
    vi.mocked(listSources).mockResolvedValue([
      {
        id: "codex-session",
        adapter_type: "codex_session",
        display_name: "Codex Sessions",
        path_or_endpoint: "/sanitized/codex",
        enabled: true,
        detected_version: "jsonl",
        health_status: "healthy",
        last_success_at: "2026-07-26T08:00:00Z",
        last_error: null,
        created_at: "2026-07-26T07:00:00Z",
        updated_at: "2026-07-26T08:00:00Z",
      },
    ]);
    window.history.pushState({}, "", "/sources");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Codex Sessions")).toBeInTheDocument();
    });
    expect(screen.getByText("/sanitized/codex")).toBeInTheDocument();

    vi.mocked(listSources).mockRejectedValue(new Error("核心不可用"));
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/无法读取数据源状态/)).toBeInTheDocument();
    });
  });

  it("does not reserve a detection row before any source is checked", async () => {
    const { container } = render(<App />);

    await screen.findByRole("button", { name: "检测 Codex" });
    expect(
      container.querySelector(".source-detections"),
    ).not.toBeInTheDocument();
  });

  it("reports each detection independently", async () => {
    vi.mocked(detectCodexPath).mockResolvedValue({
      source_id: "codex-session",
      detected: true,
      path_or_endpoint: "/sanitized/codex",
      detected_version: "jsonl",
      message: null,
    });
    vi.mocked(detectClaudePath).mockRejectedValue(new Error("路径无效"));
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "检测 Codex" }));
    await waitFor(() => {
      expect(
        screen.getByText("已检测到 Codex Session 目录"),
      ).toBeInTheDocument();
    });

    // A failing detection surfaces as an error, and does not erase the previous
    // successful result.
    fireEvent.click(screen.getByRole("button", { name: "检测 Claude" }));
    await waitFor(() => {
      expect(screen.getByText(/无法检测 Claude Home/)).toBeInTheDocument();
    });
    expect(screen.getByText("Codex Detected")).toBeInTheDocument();
  });

  it("scans every source independently and reports partial failure", async () => {
    const report = {
      inserted_events: 3,
      duplicate_events: 0,
      upserted_sessions: 1,
      updated_cursors: 1,
      skipped_records: 2,
      warning: null,
    };
    vi.mocked(rescanCodex).mockResolvedValue(report);
    vi.mocked(rescanClaude).mockResolvedValue({
      ...report,
      inserted_events: 1,
    });
    vi.mocked(rescanCcSwitch).mockRejectedValue(new Error("数据库被占用"));
    vi.mocked(rescanCockpit).mockResolvedValue({
      ...report,
      inserted_events: 0,
      skipped_records: 0,
      warning: "Cockpit 请求日志为空",
    });
    render(<App />);

    fireEvent.click(
      await screen.findByRole("button", { name: "扫描全部来源" }),
    );

    // The two successful sources are still counted; the failure is named rather
    // than discarding the whole scan.
    await waitFor(() => {
      expect(
        screen.getByText("扫描完成：新增 4 条事件，校正 0 条，跳过 4 条记录"),
      ).toBeInTheDocument();
    });
    const problems = screen.getByText(/CC-Switch 扫描失败/);
    expect(problems).toHaveTextContent("数据库被占用");
    expect(problems).toHaveTextContent("Cockpit 请求日志为空");
  });

  it("exports through the browser download path and reports failures", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(false);
    vi.mocked(exportUsage).mockResolvedValue({
      filename: "usage.csv",
      mime_type: "text/csv;charset=utf-8",
      content: "occurred_at\n",
    });
    const createObjectURL = vi.fn(() => "blob:usage");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", { ...URL, createObjectURL, revokeObjectURL });
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "导出 CSV" }));
    await waitFor(() => {
      expect(screen.getByText("已导出 usage.csv")).toBeInTheDocument();
    });
    expect(createObjectURL).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:usage");

    vi.mocked(exportUsage).mockRejectedValue(new Error("磁盘已满"));
    fireEvent.click(screen.getByRole("button", { name: "导出 JSON" }));
    await waitFor(() => {
      expect(screen.getByText(/无法导出 JSON/)).toBeInTheDocument();
    });
    vi.unstubAllGlobals();
  });

  it("writes the export to disk in the desktop shell instead of downloading", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(true);
    vi.mocked(saveExport).mockResolvedValue(
      "/Users/fixture/Downloads/usage.json",
    );
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "导出 JSON" }));

    await waitFor(() => {
      expect(
        screen.getByText("已导出到 /Users/fixture/Downloads/usage.json"),
      ).toBeInTheDocument();
    });
    expect(exportUsage).not.toHaveBeenCalled();
  });

  it("opens the local web panel and reports why it could not start", async () => {
    vi.mocked(openLocalWebApi).mockResolvedValue({
      running: true,
      url: "http://127.0.0.1:5173",
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "本地网页" }));
    await waitFor(() => {
      expect(
        screen.getByText("本地网页面板已启动：http://127.0.0.1:5173"),
      ).toBeInTheDocument();
    });

    vi.mocked(openLocalWebApi).mockRejectedValue(new Error("端口被占用"));
    fireEvent.click(screen.getByRole("button", { name: "本地网页" }));
    await waitFor(() => {
      expect(screen.getByText(/无法启动本地网页面板/)).toBeInTheDocument();
    });
  });

  it("saves settings and surfaces a rejected save", async () => {
    vi.mocked(updateAppSettings).mockImplementation(
      async (settings) => settings,
    );
    window.history.pushState({}, "", "/settings");
    render(<App />);

    const codexHome = await screen.findByLabelText("Codex Home");
    fireEvent.change(codexHome, { target: { value: "/sanitized/codex" } });
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(screen.getByText("设置已保存")).toBeInTheDocument();
    });
    expect(updateAppSettings).toHaveBeenCalledWith(
      expect.objectContaining({ codex_home: "/sanitized/codex" }),
    );

    vi.mocked(updateAppSettings).mockRejectedValue(new Error("数据库只读"));
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => {
      expect(screen.getByText(/设置保存失败/)).toBeInTheDocument();
    });
  });

  it("says the settings could not be read rather than showing blank defaults", async () => {
    vi.mocked(getAppSettings).mockRejectedValue(new Error("核心不可用"));
    window.history.pushState({}, "", "/settings");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("无法读取 Core 设置。")).toBeInTheDocument();
    });
    expect(screen.getByText("设置不可用")).toBeInTheDocument();
  });

  it("shows providers with their aggregates", async () => {
    vi.mocked(listProviders).mockResolvedValue([
      {
        id: "openai",
        provider_family: "openai",
        display_name: "OpenAI",
        upstream_url: "https://api.openai.com/v1",
        launcher: "direct",
        source_id: "codex-session",
        account_count: 1,
        request_count: 2,
        successful_request_count: 2,
        success_rate_percent: 100,
        average_latency_ms: 320,
        totals,
      },
    ]);
    window.history.pushState({}, "", "/providers");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("OpenAI")).toBeInTheDocument();
    });
    expect(screen.getByText("https://api.openai.com/v1")).toBeInTheDocument();
  });

  it("shows a quota snapshot with the precision it was recorded at", async () => {
    vi.mocked(listQuotaSnapshots).mockResolvedValue([
      {
        id: "quota-1",
        account_id: "openai:chatgpt:abc",
        account_name: "user@example.com",
        provider_name: "OpenAI",
        captured_at: "2026-07-26T10:00:00Z",
        window_type: "primary_5h",
        used_percent: 12.5,
        remaining_percent: 87.5,
        reset_at: "2026-07-26T15:00:00Z",
        credits_remaining: null,
        precision: "correlated",
        raw_json: null,
      },
    ]);
    window.history.pushState({}, "", "/quotas");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("primary_5h")).toBeInTheDocument();
    });
    expect(screen.getByText("12.5% 已用")).toBeInTheDocument();
    expect(screen.getByText(/Correlated/)).toBeInTheDocument();
  });

  it("refreshes official quota through the shared Core boundary", async () => {
    vi.mocked(refreshOfficialQuota).mockResolvedValue({
      inserted_events: 0,
      duplicate_events: 0,
      reconciled_events: 0,
      upserted_sessions: 0,
      updated_cursors: 1,
      upserted_accounts: 1,
      inserted_quota_snapshots: 1,
      skipped_records: 0,
      warning: null,
    });
    window.history.pushState({}, "", "/quotas");
    render(<App />);

    const refresh = await screen.findByRole("button", {
      name: "刷新官方额度",
    });
    fireEvent.click(refresh);
    await waitFor(() => {
      expect(refreshOfficialQuota).toHaveBeenCalledTimes(1);
    });
  });

  it("keeps unavailable dashboard values out of the totals", async () => {
    vi.mocked(getDashboardSummary).mockResolvedValue({
      period_start: "2026-07-26T00:00:00Z",
      period_end: "2026-07-27T00:00:00Z",
      totals: {
        ...totals,
        // One event lacked the field, so the total is unknown — it must not be
        // rendered as 0.
        cache_read_tokens: null,
        cache_hit_rate_percent: null,
      },
    });
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("数据已从本地 SQLite 加载")).toBeInTheDocument();
    });
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
  });

  it("reports a dashboard query failure", async () => {
    vi.mocked(getDashboardSummary).mockRejectedValue(new Error("核心不可用"));
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("无法读取本地数据层")).toBeInTheDocument();
    });
  });
});

/**
 * The tray popover is the surface most users see most often, and the one with
 * the strictest contract: it may only read the Core's pre-aggregated summary.
 */
describe("Quick summary panel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isDesktopRuntime).mockReturnValue(true);
    window.history.pushState({}, "", "/quick");
  });

  afterEach(() => {
    window.history.pushState({}, "", "/");
  });

  it("shows the official quota window and the newest warning", async () => {
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_id: "codex-session:abc",
      active_session_title: "Fixture session",
      active_project_path: "/sanitized/project",
      provider_name: "OpenAI",
      model: "gpt-5-codex",
      session_input_tokens: 100,
      session_cache_read_tokens: 20,
      session_output_tokens: 40,
      session_cache_hit_rate: 20,
      today_total_tokens: 140,
      quota_summary: {
        window_type: "primary_5h",
        used_percent: 18.75,
        remaining_percent: 81.25,
        reset_at: "2026-07-26T15:00:00Z",
        credits_remaining: null,
        precision: "correlated",
      },
      latest_warning: "Cockpit 请求日志为空",
    });
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Cockpit 请求日志为空")).toBeInTheDocument();
    });
    expect(screen.getByText(/primary_5h/)).toBeInTheDocument();
    expect(screen.getByText(/18\.8%|18\.75%/)).toBeInTheDocument();
  });

  it("states what is unknown instead of showing zeros", async () => {
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "idle",
      active_app: null,
      active_session_id: null,
      active_session_title: null,
      active_project_path: null,
      provider_name: null,
      model: null,
      session_input_tokens: null,
      session_cache_read_tokens: null,
      session_output_tokens: null,
      session_cache_hit_rate: null,
      today_total_tokens: null,
      quota_summary: null,
      latest_warning: null,
    });
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("待机")).toBeInTheDocument();
    });
    // Nothing collected yet is "Unavailable", never 0.
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    expect(screen.queryByText("0")).not.toBeInTheDocument();
  });

  it("reports a failed summary read", async () => {
    vi.mocked(getQuickSummary).mockRejectedValue(new Error("核心不可用"));
    render(<App />);

    // The failure text is the cause itself, so the popover says what went wrong
    // rather than rendering an empty panel.
    await waitFor(() => {
      expect(screen.getByText(/无法读取后台 Core/)).toHaveTextContent(
        "核心不可用",
      );
    });
  });

  it("opens the local web panel from the tray without throwing on failure", async () => {
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_id: "codex-session:abc",
      active_session_title: "Fixture session",
      active_project_path: "/sanitized/project",
      provider_name: "OpenAI",
      model: "gpt-5-codex",
      session_input_tokens: 100,
      session_cache_read_tokens: 20,
      session_output_tokens: 40,
      session_cache_hit_rate: 20,
      today_total_tokens: 140,
      quota_summary: null,
      latest_warning: null,
    });
    vi.mocked(openLocalWebApi).mockRejectedValue(new Error("端口被占用"));
    render(<App />);

    const openWeb = await screen.findByRole("button", {
      name: "打开本地网页面板…",
    });
    fireEvent.click(openWeb);

    // A failure here is logged, not thrown into the popover's render path.
    await waitFor(() => {
      expect(openLocalWebApi).toHaveBeenCalled();
    });
    expect(screen.getByText("Fixture session")).toBeInTheDocument();
  });
});

/**
 * Filters, navigation, and the remaining source controls — the interactions
 * that decide *which* numbers the panel is showing.
 */
describe("Dashboard filters and navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getDashboardSummary).mockResolvedValue({
      period_start: "2026-07-26T00:00:00Z",
      period_end: "2026-07-27T00:00:00Z",
      totals,
    });
    vi.mocked(getModelBreakdown).mockResolvedValue([]);
    vi.mocked(listSessions).mockResolvedValue({ sessions: [], total: 0 });
    vi.mocked(listSources).mockResolvedValue([]);
    vi.mocked(getSessionDetail).mockResolvedValue(null);
    vi.mocked(isDesktopRuntime).mockReturnValue(false);
  });

  afterEach(() => {
    window.history.pushState({}, "", "/");
  });

  it("re-queries with every filter the user narrows by", async () => {
    render(<App />);
    await waitFor(() => {
      expect(getDashboardSummary).toHaveBeenCalled();
    });

    fireEvent.change(screen.getByLabelText("Model"), {
      target: { value: "gpt-5-codex" },
    });
    fireEvent.change(screen.getByLabelText("Account ID"), {
      target: { value: "openai:local" },
    });
    fireEvent.change(screen.getByLabelText("搜索"), {
      target: { value: "fixture" },
    });

    // The metric cards and the session list must be narrowed by the same
    // filters, or the two halves of the screen would disagree.
    await waitFor(() => {
      expect(getDashboardSummary).toHaveBeenLastCalledWith(
        expect.objectContaining({
          model: "gpt-5-codex",
          account_id: "openai:local",
          search: "fixture",
        }),
      );
    });
    expect(listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ model: "gpt-5-codex" }),
    );

    fireEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    await waitFor(() => {
      expect(getDashboardSummary).toHaveBeenLastCalledWith(
        expect.objectContaining({
          model: null,
          account_id: null,
          search: null,
        }),
      );
    });
  });

  it("keeps the source path fields editable", async () => {
    render(<App />);

    const codexHome = await screen.findByLabelText("Codex Home");
    fireEvent.change(codexHome, { target: { value: "/sanitized/codex" } });
    expect(codexHome).toHaveValue("/sanitized/codex");

    for (const [label, value] of [
      ["Claude Home", "/sanitized/claude"],
      ["CC-Switch DB", "/sanitized/cc.db"],
      ["Cockpit DB", "/sanitized/cockpit.sqlite"],
    ] as const) {
      const field = screen.getByLabelText(label);
      fireEvent.change(field, { target: { value } });
      expect(field).toHaveValue(value);
    }

    // The typed path is what the scan uses, not the stored default.
    vi.mocked(rescanCodex).mockResolvedValue({
      inserted_events: 0,
      duplicate_events: 0,
      upserted_sessions: 0,
      updated_cursors: 0,
      skipped_records: 0,
      warning: null,
    });
    vi.mocked(rescanClaude).mockResolvedValue({
      inserted_events: 0,
      duplicate_events: 0,
      upserted_sessions: 0,
      updated_cursors: 0,
      skipped_records: 0,
      warning: null,
    });
    vi.mocked(rescanCcSwitch).mockResolvedValue({
      inserted_events: 0,
      duplicate_events: 0,
      upserted_sessions: 0,
      updated_cursors: 0,
      skipped_records: 0,
      warning: null,
    });
    vi.mocked(rescanCockpit).mockResolvedValue({
      inserted_events: 0,
      duplicate_events: 0,
      upserted_sessions: 0,
      updated_cursors: 0,
      skipped_records: 0,
      warning: null,
    });
    fireEvent.click(screen.getByRole("button", { name: "扫描全部来源" }));
    await waitFor(() => {
      expect(rescanCodex).toHaveBeenCalledWith("/sanitized/codex");
    });
    expect(rescanCockpit).toHaveBeenCalledWith("/sanitized/cockpit.sqlite");
  });

  it("opens a session's detail from the list", async () => {
    const session = {
      session: {
        id: "codex-session:abc",
        external_session_id: "abc",
        parent_session_id: null,
        app: "codex" as const,
        launcher: "direct" as const,
        project_path: "/sanitized/project",
        title: "Fixture session",
        started_at: "2026-07-26T08:00:00Z",
        ended_at: "2026-07-26T08:10:00Z",
        source_id: "codex-session",
        created_at: "2026-07-26T08:00:00Z",
        updated_at: "2026-07-26T08:10:00Z",
      },
      totals,
    };
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session],
      total: 1,
    });
    vi.mocked(getSessionDetail).mockResolvedValue({
      summary: session,
      usage_events: [],
    });
    render(<App />);

    fireEvent.click(await screen.findByText("Fixture session"));

    await waitFor(() => {
      expect(getSessionDetail).toHaveBeenCalledWith("codex-session:abc");
    });
  });

  it("explains the browser preview instead of showing a Tauri error", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(false);
    vi.mocked(getDashboardSummary).mockRejectedValue(new Error("no ipc"));
    render(<App />);

    await waitFor(() => {
      expect(
        screen.getByText("请通过 Tauri 启动以连接本地数据层"),
      ).toBeInTheDocument();
    });
    expect(screen.getByText(/浏览器预览没有 Tauri IPC/)).toBeInTheDocument();
  });

  it("navigates between routes through the nav links", async () => {
    render(<App />);
    await screen.findByRole("navigation", { name: "主要导航" });

    fireEvent.click(screen.getByRole("link", { name: "数据源" }));
    await waitFor(() => {
      expect(window.location.pathname).toBe("/sources");
    });

    // A modified click is left to the browser so "open in new tab" still works.
    const providers = screen.getByRole("link", { name: "Providers" });
    fireEvent.click(providers, { metaKey: true });
    expect(window.location.pathname).toBe("/sources");
  });
});

describe("Quick panel window fitting", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(fitQuickWindowToContent).mockResolvedValue();
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_id: "codex-session:abc",
      active_session_title: "Fixture session",
      active_project_path: "/sanitized/project",
      provider_name: "OpenAI",
      model: "gpt-5-codex",
      session_input_tokens: 100,
      session_cache_read_tokens: 20,
      session_output_tokens: 40,
      session_cache_hit_rate: 20,
      today_total_tokens: 140,
      quota_summary: null,
      latest_warning: null,
    });
    window.history.pushState({}, "", "/quick");
  });

  afterEach(() => {
    window.history.pushState({}, "", "/");
  });

  it("asks the window to match the rendered content height", async () => {
    // The popover window is created at a fixed height because Rust cannot know
    // how tall the summary will be; whatever the content does not fill would
    // otherwise show as dead space under the last row.
    const shellHeight = 372;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      height: shellHeight,
      width: 320,
      top: 0,
      left: 0,
      right: 320,
      bottom: shellHeight,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });

    render(<App />);

    await waitFor(() => {
      expect(fitQuickWindowToContent).toHaveBeenCalledWith(shellHeight);
    });
    vi.restoreAllMocks();
  });
});
