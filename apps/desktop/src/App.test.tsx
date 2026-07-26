import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import App from "./App";
import {
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
  pickDirectory,
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
  listAccounts: vi.fn(),
  listProviders: vi.fn(),
  listQuotaSnapshots: vi.fn(),
  listSessions: vi.fn(),
  listSources: vi.fn(),
  openLocalWebApi: vi.fn(),
  pickDirectory: vi.fn(),
  pickFile: vi.fn(),
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
