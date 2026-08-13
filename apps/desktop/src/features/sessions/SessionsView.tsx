import { useEffect, useState } from "react";

import { listSessions, type SessionSummary } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, SessionRow } from "../../components/Presentation";
import { describeError } from "../../lib/format";
import { navigate } from "../../lib/navigation";

export function SessionsView() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listSessions({}, 100, 0)
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
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame>
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section
        className="panel sessions-panel route-panel"
        aria-label="会话列表"
      >
        <div className="sessions-count-row">
          <span className="count-label">
            {sessions.length} 条
            {total != null && total > sessions.length ? ` / 共 ${total}` : ""}
          </span>
        </div>
        {sessions.length ? (
          <div className="session-list">
            {sessions.map((session) => (
              <SessionRow
                key={session.session.id}
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
        ) : (
          <EmptyState
            title="还没有导入会话"
            description="在总览页点击“扫描全部来源”，或在设置页配置各数据源路径后保存，TokenBuddy 会开始增量导入。"
          />
        )}
      </section>
    </PageFrame>
  );
}
