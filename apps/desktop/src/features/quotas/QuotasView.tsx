import { useEffect, useState, type CSSProperties } from "react";

import {
  listAccounts,
  listQuotaSnapshots,
  listSources,
  refreshOfficialQuota,
  type AccountSummary,
  type QuotaSummary,
  type QuotaSnapshot,
  type SourceRecord,
} from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { EmptyState, Meter, Notice } from "../../components/Presentation";
import {
  authModeLabel,
  formatDate,
  formatPercent,
  precisionLabel,
} from "../../lib/format";
import { toast } from "../../lib/toast";

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

// Credits windows have no percentage; drawing them as a 0% bar would claim an
// untouched quota, so they get no gauge at all.
function quotaMeterPercent(quota: QuotaSnapshot | QuotaSummary | null) {
  if (!quota || quota.window_type === "credits") return null;
  return quota.used_percent;
}

const channels = [
  [
    "openai-official-quota",
    "Codex / ChatGPT",
    "读取 Codex Home 中的官方登录态；后台最多每 5 分钟请求一次，API Key 不提供订阅额度。",
  ],
  [
    "claude-official-quota",
    "Claude Code",
    "读取官方 statusLine 的 5 小时、7 天窗口；按会话范围展示，不推断账号或订阅方案。",
  ],
] as const;

const claudeCaptureCommand =
  '"TokenBuddy 可执行文件的完整路径" --capture-claude-quota "Claude Home 完整路径"';

export function QuotasView() {
  const [quotas, setQuotas] = useState<QuotaSnapshot[]>([]);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let active = true;
    void Promise.all([listQuotaSnapshots(), listAccounts(), listSources()])
      .then(([nextQuotas, nextAccounts, nextSources]) => {
        if (active) {
          setQuotas(nextQuotas);
          setAccounts(nextAccounts);
          setSources(nextSources);
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
      const report = await refreshOfficialQuota();
      const [nextQuotas, nextAccounts, nextSources] = await Promise.all([
        listQuotaSnapshots(),
        listAccounts(),
        listSources(),
      ]);
      setQuotas(nextQuotas);
      setAccounts(nextAccounts);
      setSources(nextSources);
      setError(report?.warning ?? null);
      if (!report?.warning) toast("官方额度已刷新");
    } catch (cause) {
      console.error("刷新官方额度失败", cause);
      setError(
        cause instanceof Error
          ? cause.message
          : "官方额度刷新失败，请检查 Codex 登录或 Claude 状态栏采集入口。",
      );
    } finally {
      setIsRefreshing(false);
    }
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(claudeCaptureCommand);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch (cause) {
      console.error("复制命令失败", cause);
    }
  }

  return (
    <PageFrame
      busy={isRefreshing}
      actions={
        <>
          <span className="quota-sync-status">
            {isRefreshing
              ? "正在同步官方窗口"
              : quotas.length
                ? `已保存 ${quotas.length} 条快照`
                : "等待官方窗口"}
          </span>
          <button
            className="primary-button"
            type="button"
            onClick={() => void handleRefresh()}
            disabled={isRefreshing}
          >
            {isRefreshing ? "刷新中…" : "刷新官方额度"}
          </button>
        </>
      }
    >
      {error ? <Notice onDismiss={() => setError(null)}>{error}</Notice> : null}

      <section className="panel quota-panel" aria-label="官方订阅渠道">
        <div className="quota-section-heading">
          <div>
            <p className="section-kicker">Channels</p>
            <h2>官方订阅渠道</h2>
            <p className="quota-section-description">
              订阅窗口独立于 Token 统计；API
              价格估算不代表订阅实际扣费，也不能换算剩余 Token。
            </p>
          </div>
        </div>
        <div className="channel-grid">
          {channels.map(([id, label, description]) => {
            const source = sources.find((item) => item.id === id);
            const state =
              source?.health_status === "healthy"
                ? "ok"
                : source?.health_status === "stale"
                  ? "stale"
                  : "missing";
            return (
              <article className="channel-card" key={id} data-glow>
                <div className="channel-heading">
                  <h3>{label}</h3>
                  <span
                    className={`health-badge ${state === "missing" ? "missing" : state}`}
                  >
                    {state === "ok"
                      ? "已接入"
                      : state === "stale"
                        ? "数据已过期，等待新快照"
                        : "待接入或暂不可用"}
                  </span>
                </div>
                <p className="channel-description">{description}</p>
                <p className="channel-meta">
                  最近成功：
                  {source?.last_success_at
                    ? formatDate(source.last_success_at)
                    : "Unavailable"}
                </p>
                {source?.last_error ? (
                  <p className="card-error">{source.last_error}</p>
                ) : null}
              </article>
            );
          })}
        </div>
        <details className="setup-details">
          <summary>接入 Claude Code 官方订阅</summary>
          <div className="setup-body">
            <p>
              在 Claude Code 的 statusLine command 中调用 TokenBuddy
              可执行文件，并添加参数 <code>--capture-claude-quota</code>。自定义
              Claude Home 时，再传入该目录作为第二个参数。
            </p>
            <div className="code-block">
              <pre>{claudeCaptureCommand}</pre>
              <button
                className="quiet-button"
                type="button"
                onClick={() => void copyCommand()}
              >
                {copied ? "已复制" : "复制"}
              </button>
            </div>
            <p>
              此入口只保存额度数值、时间和会话指纹，不启动面板。已有状态栏需要合并处理同一份标准输入，不能直接串联两个读取
              stdin 的命令。TokenBuddy 不会覆盖你的 Claude 配置。
            </p>
            <a
              href="https://code.claude.com/docs/en/statusline"
              target="_blank"
              rel="noreferrer"
            >
              Claude 官方状态栏配置说明 ↗
            </a>
          </div>
        </details>
      </section>

      <section
        className="panel quota-panel quota-accounts-panel"
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
        </div>

        {accounts.length ? (
          <div className="quota-account-list">
            {accounts.map(({ account, provider_name, latest_quota }, index) => {
              const windows = quotas
                .filter((q) => q.account_id === account.id)
                .filter(
                  (q, windowIndex, all) =>
                    !all
                      .slice(0, windowIndex)
                      .some(
                        (previous) => previous.window_type === q.window_type,
                      ),
                );
              return (
                <article
                  className={quotaStateClass(latest_quota)}
                  key={account.id}
                  data-glow
                  style={{ "--i": index } as CSSProperties}
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

                  <div className="quota-account-main">
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
                    {latest_quota && latest_quota.window_type !== "credits" ? (
                      <Meter
                        percent={quotaMeterPercent(latest_quota)}
                        label={`${quotaWindowValue(latest_quota)} 已用`}
                      />
                    ) : null}
                  </div>

                  <div className="quota-account-foot">
                    <span className="precision-badge">
                      {quotaPrecisionValue(latest_quota)}
                    </span>
                    <span>指纹 {account.account_fingerprint.slice(0, 12)}</span>
                  </div>

                  {windows.length ? (
                    <div className="quota-window-list">
                      {windows.map((q) => {
                        const expired =
                          q.reset_at != null &&
                          Date.parse(q.reset_at) <= Date.now();
                        return (
                          <div
                            className="quota-window-row"
                            key={q.window_type}
                            data-expired={expired || undefined}
                          >
                            <span className="quota-window-name">
                              {q.window_type}
                            </span>
                            {q.window_type === "credits" ? (
                              <span className="quota-window-credits">
                                余额 {quotaRemainingValue(q)}
                              </span>
                            ) : (
                              <Meter
                                percent={quotaMeterPercent(q)}
                                label={`${q.window_type} 已用`}
                              />
                            )}
                            <span className="quota-window-text">
                              已用 {quotaUsedValue(q)} · 剩余{" "}
                              {quotaRemainingValue(q)} · 重置{" "}
                              {quotaResetValue(q)} · 采集于{" "}
                              {formatDate(q.captured_at)}
                              {expired ? "（窗口已到期，等待更新）" : ""}
                            </span>
                          </div>
                        );
                      })}
                    </div>
                  ) : null}
                </article>
              );
            })}
          </div>
        ) : (
          <EmptyState
            compact
            title="账号 Unavailable"
            description="尚未识别到任何账号；Codex 官方账号需要可读的 auth.json。"
          />
        )}
      </section>

      {quotas.length ? (
        <section
          className="panel quota-panel quota-snapshots-panel"
          aria-label="官方额度快照"
        >
          <div className="quota-section-heading">
            <div>
              <p className="section-kicker">Snapshots</p>
              <h2>额度快照</h2>
              <p className="quota-section-description">
                窗口变化时保存快照；重复刷新不增加记录。历史值不代表当前剩余额度。
              </p>
            </div>
            <span className="count-label">{quotas.length} 条</span>
          </div>
          <div className="quota-snapshot-list">
            {quotas.map((quota, index) => (
              <article
                className="quota-snapshot-card"
                key={quota.id}
                style={{ "--i": Math.min(index, 14) } as CSSProperties}
              >
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
                  {quota.window_type !== "credits" ? (
                    <Meter
                      percent={quotaMeterPercent(quota)}
                      label={`${quota.window_type} 已用`}
                    />
                  ) : null}
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
        <section className="panel quota-panel">
          <EmptyState
            compact
            title="官方额度 Unavailable"
            description="尚未连接官方额度数据源；此处不会使用 Session Token 估算订阅额度。"
          />
        </section>
      )}
    </PageFrame>
  );
}
