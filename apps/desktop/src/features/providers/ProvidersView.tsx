import { useEffect, useState, type CSSProperties } from "react";

import { listProviders, type ProviderSummary } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import {
  EmptyState,
  Meter,
  Notice,
  SummaryItem,
} from "../../components/Presentation";
import { formatCost, formatPercent, formatTokens } from "../../lib/format";

export function ProvidersView() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let active = true;
    void listProviders()
      .then((nextProviders) => {
        if (active) {
          setProviders(nextProviders);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取 Provider 统计。");
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame>
      {error ? <Notice>{error}</Notice> : null}
      {providers.length ? (
        <section className="route-grid" aria-label="Provider 统计">
          {providers.map((provider, index) => (
            <article
              className="panel route-card"
              key={provider.id}
              data-glow
              style={{ "--i": index } as CSSProperties}
            >
              <div className="route-card-heading">
                <div>
                  <p className="section-kicker">{provider.provider_family}</p>
                  <h2>{provider.display_name}</h2>
                </div>
                <span className="count-label">
                  {formatTokens(provider.request_count)} 请求
                </span>
              </div>
              <div className="card-stats">
                <div>
                  <span>输入</span>
                  <strong>
                    {formatTokens(provider.totals.input_tokens_total)}
                  </strong>
                </div>
                <div>
                  <span>输出</span>
                  <strong>
                    {formatTokens(provider.totals.output_tokens_total)}
                  </strong>
                </div>
                <div>
                  <span>费用（USD）</span>
                  <strong>{formatCost(provider.totals)}</strong>
                </div>
              </div>
              <div className="card-meter">
                <span>
                  成功率
                  <strong>
                    {formatPercent(provider.success_rate_percent)}
                  </strong>
                </span>
                <Meter
                  percent={provider.success_rate_percent}
                  label="成功率"
                  tone="accent"
                />
              </div>
              <dl className="summary-list">
                <SummaryItem
                  label="上游 URL"
                  value={provider.upstream_url || "Unavailable"}
                />
                <SummaryItem
                  label="账号数"
                  value={formatTokens(provider.account_count)}
                />
                <SummaryItem
                  label="平均延迟"
                  value={
                    provider.average_latency_ms == null
                      ? "Unavailable"
                      : `${provider.average_latency_ms.toFixed(0)} ms`
                  }
                />
                <SummaryItem
                  label="缓存命中率"
                  value={formatPercent(provider.totals.cache_hit_rate_percent)}
                />
              </dl>
            </article>
          ))}
        </section>
      ) : isLoading ? (
        <section className="route-grid" aria-label="正在读取 Provider">
          <div className="panel route-card skeleton-card" />
          <div className="panel route-card skeleton-card" />
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="Provider 数据 Unavailable"
            description="当前已导入 Codex 与 Claude Code Session；Provider Adapter 尚未提供可验证归属。"
          />
        </section>
      )}
    </PageFrame>
  );
}
