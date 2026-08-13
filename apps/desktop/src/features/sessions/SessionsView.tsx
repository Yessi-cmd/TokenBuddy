import { useEffect, useState } from "react";

import { listSessions, type SessionSummary } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, SessionRow } from "../../components/Presentation";
import { describeError } from "../../lib/format";
import { navigate } from "../../lib/navigation";

export function SessionsView() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listSessions({}, 100, 0)
      .then((page) => {
        if (active) {
          setSessions(page.sessions);
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
            description="确认 Codex Home 后开始增量导入。"
          />
        )}
      </section>
    </PageFrame>
  );
}
