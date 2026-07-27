import { useEffect, useState } from "react";

import { listSources, type SourceRecord } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, SummaryItem } from "../../components/Presentation";
import { formatDate } from "../../lib/format";

export function SourcesView() {
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listSources()
      .then((nextSources) => {
        if (active) {
          setSources(nextSources);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取数据源状态。");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Read-only adapters"
      title="数据源"
      subtitle="每个 Adapter 独立报告路径、健康状态和最近错误。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {sources.length ? (
        <section className="route-grid" aria-label="数据源状态">
          {sources.map((source) => (
            <article className="panel route-card" key={source.id}>
              <div className="panel-heading route-card-heading">
                <div>
                  <p className="section-kicker">{source.adapter_type}</p>
                  <h2>{source.display_name}</h2>
                </div>
                <span className="detection ok">
                  {source.health_status || "Unavailable"}
                </span>
              </div>
              <dl className="summary-list">
                <SummaryItem
                  label="检测路径"
                  value={source.path_or_endpoint || "Unavailable"}
                />
                <SummaryItem
                  label="版本"
                  value={source.detected_version || "Unavailable"}
                />
                <SummaryItem
                  label="最近导入"
                  value={
                    source.last_success_at
                      ? formatDate(source.last_success_at)
                      : "Unavailable"
                  }
                />
                <SummaryItem
                  label="最近错误"
                  value={source.last_error || "Unavailable"}
                />
              </dl>
            </article>
          ))}
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="尚未登记数据源"
            description="启动 Core 后会在此展示 Adapter 状态。"
          />
        </section>
      )}
    </PageFrame>
  );
}
