import { useEffect, useState } from "react";

import {
  listAccounts,
  listQuotaSnapshots,
  refreshOfficialQuota,
  type AccountSummary,
  type QuotaSummary,
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

function formatCredits(value: number | null) {
  return value == null
    ? "Unavailable"
    : new Intl.NumberFormat("zh-CN", {
        maximumFractionDigits: 2,
      }).format(value);
}

function quotaHeadline(quota: QuotaSnapshot | QuotaSummary | null) {
  if (!quota) return "额度 Unavailable";
  if (quota.window_type === "credits") {
    return `Credits 剩余 ${formatCredits(quota.credits_remaining)}`;
  }
  return `${quota.window_type} ${formatPercent(quota.used_percent)} 已用`;
}

function quotaDetail(quota: QuotaSnapshot | QuotaSummary | null) {
  if (!quota) return "该账号尚未报告官方额度窗口";
  if (quota.window_type === "credits") {
    return `额度余额 ${formatCredits(quota.credits_remaining)} · ${precisionLabel(quota.precision)}`;
  }
  return `剩余 ${formatPercent(quota.remaining_percent)} · ${precisionLabel(quota.precision)}`;
}

export function QuotasView() {
  const [quotas, setQuotas] = useState<QuotaSnapshot[]>([]);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);

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

  async function handleRefresh() {
    setIsRefreshing(true);
    try {
      await refreshOfficialQuota();
      const [nextQuotas, nextAccounts] = await Promise.all([
        listQuotaSnapshots(),
        listAccounts(),
      ]);
      setQuotas(nextQuotas);
      setAccounts(nextAccounts);
      setError(null);
    } catch (cause) {
      console.error("刷新官方额度失败", cause);
      setError(
        cause instanceof Error
          ? cause.message
          : "官方额度刷新失败，请检查 Codex 登录状态。",
      );
    } finally {
      setIsRefreshing(false);
    }
  }

  return (
    <PageFrame
      eyebrow="Official quota windows"
      title="官方额度"
      subtitle="官方额度与原始 Token 分开保存；不依赖 Cockpit，也不会从百分比反推准确 Token。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section className="panel route-panel" aria-label="已识别账号">
        <div className="settings-heading">
          <div>
            <p className="section-kicker">Accounts</p>
            <h2>已识别账号</h2>
          </div>
          <button
            className="quiet-button"
            type="button"
            onClick={() => void handleRefresh()}
            disabled={isRefreshing}
          >
            {isRefreshing ? "刷新中…" : "刷新官方额度"}
          </button>
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
                  <strong>{quotaHeadline(latest_quota)}</strong>
                  <span>
                    {latest_quota
                      ? latest_quota.window_type === "credits"
                        ? quotaDetail(latest_quota)
                        : precisionLabel(latest_quota.precision)
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
                  <strong>
                    {quota.window_type === "credits"
                      ? quotaHeadline(quota)
                      : `${formatPercent(quota.used_percent)} 已用`}
                  </strong>
                  <span>{quotaDetail(quota)}</span>
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
