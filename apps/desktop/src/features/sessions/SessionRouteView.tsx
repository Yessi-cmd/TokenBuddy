import { useEffect, useState } from "react";

import { getSessionDetail, type SessionDetail } from "../../lib/api";
import { PageFrame, RouteLink } from "../../components/Navigation";
import {
  EmptyState,
  Notice,
  SessionDetailView,
} from "../../components/Presentation";

export function SessionRouteView({ sessionId }: { sessionId: string }) {
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

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
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [sessionId]);

  return (
    <PageFrame
      actions={
        <RouteLink to="/sessions">
          <span className="back-link">← 返回会话列表</span>
        </RouteLink>
      }
    >
      {error ? <Notice>{error}</Notice> : null}
      {detail ? (
        <section className="panel detail-panel route-panel">
          <SessionDetailView detail={detail} />
        </section>
      ) : isLoading ? (
        <section className="panel detail-panel route-panel">
          <div className="detail-loading" aria-label="正在读取会话详情">
            <span className="skeleton skeleton-title" />
            <span className="skeleton skeleton-line" />
            <span className="skeleton skeleton-block" />
          </div>
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
