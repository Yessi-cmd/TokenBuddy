import { useEffect, useState } from "react";

import { listSessions, type SessionSummary } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, Notice, SessionRow } from "../../components/Presentation";
import { describeError } from "../../lib/format";
import { navigate } from "../../lib/navigation";

const PAGE_SIZE = 100;

export function SessionsView() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);

  // Typing should not fire one Core query per keystroke.
  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), 250);
    return () => clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    let active = true;
    void listSessions(query ? { search: query } : {}, PAGE_SIZE, 0)
      .then((page) => {
        if (active) {
          setSessions(page.sessions);
          setTotal(page.total);
          setError(null);
        }
      })
      .catch((cause: unknown) => {
        console.error("读取会话列表失败", cause);
        if (active) setError(`无法读取会话列表：${describeError(cause)}`);
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [query]);

  async function loadMore() {
    setIsLoadingMore(true);
    try {
      const page = await listSessions(
        query ? { search: query } : {},
        PAGE_SIZE,
        sessions.length,
      );
      setSessions((current) => [...current, ...page.sessions]);
      setTotal(page.total);
    } catch (cause) {
      console.error("读取更多会话失败", cause);
      setError(`无法读取会话列表：${describeError(cause)}`);
    } finally {
      setIsLoadingMore(false);
    }
  }

  return (
    <PageFrame>
      {error ? <Notice>{error}</Notice> : null}
      <section
        className="panel sessions-panel route-panel"
        aria-label="会话列表"
      >
        <div className="sessions-toolbar">
          <label className="scope-search">
            <span className="sr-only">搜索会话</span>
            <input
              type="search"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="搜索标题、项目、会话 ID、模型或请求 ID"
            />
          </label>
          <span className="count-label">
            {sessions.length} 条
            {total != null && total > sessions.length ? ` / 共 ${total}` : ""}
          </span>
        </div>
        {sessions.length ? (
          <div className="session-list">
            {sessions.map((session, index) => (
              <SessionRow
                key={session.session.id}
                index={index}
                summary={session}
                selected={false}
                onSelect={() =>
                  navigate(
                    `/sessions/${encodeURIComponent(session.session.id)}`,
                  )
                }
              />
            ))}
          </div>
        ) : isLoading ? (
          <div className="detail-loading" aria-label="正在读取会话">
            <span className="skeleton skeleton-line" />
            <span className="skeleton skeleton-line" />
            <span className="skeleton skeleton-line" />
          </div>
        ) : (
          <EmptyState
            title={query ? "没有匹配的会话" : "还没有导入会话"}
            description={
              query
                ? "换一个关键词，或清空搜索查看全部会话。"
                : "在总览页点击“扫描全部来源”，或在设置页配置各数据源路径后保存，TokenBuddy 会开始增量导入。"
            }
          />
        )}
        {total != null && total > sessions.length ? (
          <div className="panel-foot">
            <button
              className="quiet-button"
              type="button"
              onClick={() => void loadMore()}
              disabled={isLoadingMore}
            >
              {isLoadingMore ? "加载中…" : "加载更多"}
            </button>
          </div>
        ) : null}
      </section>
    </PageFrame>
  );
}
