import { useEffect, useState } from "react";

import { listProviders, type ProviderSummary } from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, SummaryItem } from "../../components/Presentation";
import { formatCost, formatPercent, formatTokens } from "../../lib/format";

export function ProvidersView() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

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
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Provider observatory"
      title="Providers"
      subtitle="只展示已被数据源明确识别的 Provider；无法归属时保持 Unavailable。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {providers.length ? (
        <section className="route-grid" aria-label="Provider 统计">
          {providers.map((provider) => (
            <article className="panel route-card" key={provider.id}>
              <div className="panel-heading route-card-heading">
                <div>
                  <p className="section-kicker">{provider.provider_family}</p>
                  <h2>{provider.display_name}</h2>
                </div>
                <span className="count-label">
                  {formatTokens(provider.request_count)} 请求
                </span>
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
                  label="成功率"
                  value={formatPercent(provider.success_rate_percent)}
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
                  label="输入 / 输出"
                  value={`${formatTokens(provider.totals.input_tokens_total)} / ${formatTokens(provider.totals.output_tokens_total)}`}
                />
                <SummaryItem label="费用" value={formatCost(provider.totals)} />
              </dl>
            </article>
          ))}
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
