import { useEffect, useState } from "react";

import {
  listAccounts,
  listQuotaSnapshots,
  type AccountSummary,
  type QuotaSnapshot,
} from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState } from "../../components/Presentation";
import {
  authModeLabel,
  formatDate,
  formatPercent,
  precisionLabel,
} from "../../lib/format";

export function QuotasView() {
  const [quotas, setQuotas] = useState<QuotaSnapshot[]>([]);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void Promise.all([listQuotaSnapshots(), listAccounts()])
      .then(([nextQuotas, nextAccounts]) => {
        if (active) {
          setQuotas(nextQuotas);
          setAccounts(nextAccounts);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取官方额度快照。");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Official quota windows"
      title="官方额度"
      subtitle="额度窗口与原始 Token 分开保存；不会从百分比反推准确 Token。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section className="panel route-panel" aria-label="已识别账号">
        <div className="settings-heading">
          <div>
            <p className="section-kicker">Accounts</p>
            <h2>已识别账号</h2>
          </div>
        </div>
        {accounts.length ? (
          <div className="quota-list">
            {accounts.map(({ account, provider_name, latest_quota }) => (
              <article className="quota-row" key={account.id}>
                <div>
                  <strong>{account.display_name || "账号 Unavailable"}</strong>
                  <span>
                    {provider_name || "Provider Unavailable"} ·{" "}
                    {authModeLabel(account.auth_mode)}
                  </span>
                </div>
                <div>
                  <strong>{account.plan || "订阅方案 Unavailable"}</strong>
                  <span>指纹 {account.account_fingerprint.slice(0, 12)}</span>
                </div>
                <div>
                  <strong>
                    {latest_quota
                      ? `${latest_quota.window_type} ${formatPercent(latest_quota.used_percent)} 已用`
                      : "额度 Unavailable"}
                  </strong>
                  <span>
                    {latest_quota
                      ? precisionLabel(latest_quota.precision)
                      : "该账号尚未报告官方额度窗口"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <EmptyState
            title="账号 Unavailable"
            description="尚未识别到任何账号；Codex 官方账号需要可读的 auth.json。"
          />
        )}
      </section>
      {quotas.length ? (
        <section className="panel route-panel" aria-label="官方额度快照">
          <div className="quota-list">
            {quotas.map((quota) => (
              <article className="quota-row" key={quota.id}>
                <div>
                  <strong>{quota.window_type}</strong>
                  <span>
                    {quota.provider_name || "Provider Unavailable"} ·{" "}
                    {quota.account_name || "账号 Unavailable"}
                  </span>
                </div>
                <div>
                  <strong>{formatPercent(quota.used_percent)} 已用</strong>
                  <span>
                    剩余 {formatPercent(quota.remaining_percent)} ·{" "}
                    {precisionLabel(quota.precision)}
                  </span>
                </div>
                <div>
                  <strong>{formatDate(quota.captured_at)}</strong>
                  <span>
                    重置{" "}
                    {quota.reset_at
                      ? formatDate(quota.reset_at)
                      : "Unavailable"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="官方额度 Unavailable"
            description="尚未连接官方额度数据源；此处不会使用 Session Token 估算订阅额度。"
          />
        </section>
      )}
    </PageFrame>
  );
}
