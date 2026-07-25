import { render, screen, waitFor } from "@testing-library/react";

import App from "./App";
import {
  getDashboardSummary,
  getSessionDetail,
  listSessions,
  listSources,
} from "./lib/api";

vi.mock("./lib/api", () => ({
  detectCodexPath: vi.fn(),
  getDashboardSummary: vi.fn(),
  getSessionDetail: vi.fn(),
  listSessions: vi.fn(),
  listSources: vi.fn(),
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
    vi.mocked(getDashboardSummary).mockResolvedValue({
      period_start: "2026-07-26T00:00:00Z",
      period_end: "2026-07-27T00:00:00Z",
      totals,
    });
    vi.mocked(listSessions).mockResolvedValue({ sessions: [], total: 0 });
    vi.mocked(listSources).mockResolvedValue([]);
    vi.mocked(getSessionDetail).mockResolvedValue(null);
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
});
