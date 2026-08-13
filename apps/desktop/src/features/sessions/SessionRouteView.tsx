import { useEffect, useState } from "react";

import { getSessionDetail, type SessionDetail } from "../../lib/api";
import { PageFrame, RouteLink } from "../../components/Navigation";
import { EmptyState, SessionDetailView } from "../../components/Presentation";

export function SessionRouteView({ sessionId }: { sessionId: string }) {
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void getSessionDetail(sessionId)
      .then((nextDetail) => {
        if (active) {
          setDetail(nextDetail);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取会话详情。");
      });
    return () => {
      active = false;
    };
  }, [sessionId]);

  return (
    <PageFrame>
      <p className="route-back">
        <RouteLink to="/sessions">← 返回会话列表</RouteLink>
      </p>
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {detail ? (
        <section className="panel detail-panel route-panel">
          <SessionDetailView detail={detail} />
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="会话 Unavailable"
            description="Core 没有返回该会话。"
          />
        </section>
      )}
    </PageFrame>
  );
}
