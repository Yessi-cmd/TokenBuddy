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

function quotaUsedValue(quota: QuotaSnapshot | QuotaSummary | null) {
  if (!quota || quota.window_type === "credits") return "Unavailable";
  return formatPercent(quota.used_percent);
}

function quotaRemainingValue(quota: QuotaSnapshot | QuotaSummary | null) {
  if (!quota) return "Unavailable";
  return quota.window_type === "credits"
    ? formatCredits(quota.credits_remaining)
    : formatPercent(quota.remaining_percent);
}

function quotaWindowValue(quota: QuotaSnapshot | QuotaSummary | null) {
  return quota?.window_type ?? "官方额度";
}

function quotaResetValue(quota: QuotaSnapshot | QuotaSummary | null) {
  return quota?.reset_at ? formatDate(quota.reset_at) : "Unavailable";
}

function quotaPrecisionValue(quota: QuotaSnapshot | QuotaSummary | null) {
  return quota ? precisionLabel(quota.precision) : "尚未报告";
}

function quotaStateClass(quota: QuotaSnapshot | QuotaSummary | null) {
  return quota
    ? "quota-account-card has-data"
    : "quota-account-card is-unavailable";
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
    <div className="quota-page">
      <PageFrame>
        {error ? <p className="notice notice-warning">{error}</p> : null}

        <section
          className="panel route-panel quota-panel quota-accounts-panel"
          aria-label="已识别账号"
        >
          <div className="quota-section-heading">
            <div>
              <p className="section-kicker">Accounts</p>
              <h2>已识别账号</h2>
              <p className="quota-section-description">
                官方账号与本地会话账号分开显示；没有官方窗口的账号保持
                Unavailable。
              </p>
            </div>
            <div className="quota-heading-actions">
              <span className="quota-sync-status">
                {isRefreshing
                  ? "正在同步官方窗口"
                  : quotas.length
                    ? `已保存 ${quotas.length} 条快照`
                    : "等待官方窗口"}
              </span>
              <button
                className="quiet-button"
                type="button"
                onClick={() => void handleRefresh()}
                disabled={isRefreshing}
              >
                {isRefreshing ? "刷新中…" : "刷新官方额度"}
              </button>
            </div>
          </div>

          {accounts.length ? (
            <div className="quota-account-list">
              {accounts.map(({ account, provider_name, latest_quota }) => (
                <article
                  className={quotaStateClass(latest_quota)}
                  key={account.id}
                >
                  <div className="quota-account-identity">
                    <strong>
                      {account.display_name || "账号 Unavailable"}
                    </strong>
                    <span>
                      {provider_name || "Provider Unavailable"} ·{" "}
                      {authModeLabel(account.auth_mode)}
                    </span>
                    <span>{account.plan || "订阅方案 Unavailable"}</span>
                  </div>

                  <div className="quota-account-metrics">
                    <div className="quota-metric">
                      <span>窗口 · {quotaWindowValue(latest_quota)}</span>
                      <strong>{quotaUsedValue(latest_quota)}</strong>
                      <small>已用</small>
                    </div>
                    <div className="quota-metric">
                      <span>剩余</span>
                      <strong>{quotaRemainingValue(latest_quota)}</strong>
                      <small>
                        {latest_quota?.window_type === "credits"
                          ? "Credits"
                          : "官方返回"}
                      </small>
                    </div>
                    <div className="quota-metric quota-metric-reset">
                      <span>重置</span>
                      <strong>{quotaResetValue(latest_quota)}</strong>
                      <small>{latest_quota ? "官方返回" : "尚未报告"}</small>
                    </div>
                  </div>

                  <div className="quota-account-foot">
                    <span className="precision-badge">
                      {quotaPrecisionValue(latest_quota)}
                    </span>
                    <span>指纹 {account.account_fingerprint.slice(0, 12)}</span>
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
          <section
            className="panel route-panel quota-panel quota-snapshots-panel"
            aria-label="官方额度快照"
          >
            <div className="quota-section-heading quota-snapshots-heading">
              <div>
                <p className="section-kicker">Snapshots</p>
                <h2>额度快照</h2>
                <p className="quota-section-description">
                  每次刷新都会保留一条官方窗口记录，便于查看使用变化。
                </p>
              </div>
              <span className="quota-count-label">{quotas.length} 条</span>
            </div>
            <div className="quota-snapshot-list">
              {quotas.map((quota) => (
                <article className="quota-snapshot-card" key={quota.id}>
                  <div className="quota-snapshot-identity">
                    <span className="quota-window-badge">
                      {quota.window_type}
                    </span>
                    <strong>{quota.account_name || "账号 Unavailable"}</strong>
                    <span>
                      {quota.provider_name || "Provider Unavailable"} · 采集于{" "}
                      {formatDate(quota.captured_at)}
                    </span>
                  </div>
                  <div className="quota-metric">
                    <span>已用</span>
                    <strong>{quotaUsedValue(quota)}</strong>
                  </div>
                  <div className="quota-metric">
                    <span>剩余</span>
                    <strong>{quotaRemainingValue(quota)}</strong>
                  </div>
                  <div className="quota-metric quota-metric-reset">
                    <span>重置</span>
                    <strong>{quotaResetValue(quota)}</strong>
                  </div>
                  <span className="precision-badge">
                    {precisionLabel(quota.precision)}
                  </span>
                </article>
              ))}
            </div>
          </section>
        ) : (
          <section className="panel route-panel quota-panel">
            <EmptyState
              title="官方额度 Unavailable"
              description="尚未连接官方额度数据源；此处不会使用 Session Token 估算订阅额度。"
            />
          </section>
        )}
      </PageFrame>
    </div>
  );
}
