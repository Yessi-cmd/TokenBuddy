import { render, screen, waitFor } from "@testing-library/react";

import App from "./App";
import {
  getDashboardSummary,
  getQuickSummary,
  getSessionDetail,
  listSessions,
  listSources,
} from "./lib/api";

vi.mock("./lib/api", () => ({
  detectCodexPath: vi.fn(),
  getDashboardSummary: vi.fn(),
  getQuickSummary: vi.fn(),
  getSessionDetail: vi.fn(),
  listSessions: vi.fn(),
  listSources: vi.fn(),
  openLocalWebApi: vi.fn(),
  rescanCodex: vi.fn(),
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
    vi.mocked(listSessions).mockResolvedValue({ sessions: [], total: 0 });
    vi.mocked(listSources).mockResolvedValue([]);
    vi.mocked(getSessionDetail).mockResolvedValue(null);
    vi.mocked(getQuickSummary).mockResolvedValue({
      collection_status: "collecting",
      active_app: "codex",
      active_session_title: "Fixture session",
      provider_name: null,
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
      screen.getByRole("button", { name: "扫描 Codex" }),
    ).toBeInTheDocument();
    expect(screen.getByText("输入 Token")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("数据已从本地 SQLite 加载")).toBeInTheDocument();
    });
    expect(screen.getByText("100")).toBeInTheDocument();
    expect(screen.getByText("25.0%")).toBeInTheDocument();
  });

  it("renders the tray quick view from QuickSummary without loading sessions", async () => {
    window.history.pushState({}, "", "/quick");
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("今日 Token")).toBeInTheDocument();
    });
    expect(screen.getByText("140")).toBeInTheDocument();
    expect(screen.getByText("采集中")).toBeInTheDocument();
    expect(getQuickSummary).toHaveBeenCalled();
    expect(listSessions).not.toHaveBeenCalled();
  });
});
