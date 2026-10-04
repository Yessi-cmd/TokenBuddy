import { useEffect, useState, type CSSProperties } from "react";

import {
  detectCcSwitchPath,
  detectClaudePath,
  detectCockpitPath,
  detectCodexPath,
  detectDshPath,
  detectOfficialQuotaPath,
  detectOpenCodePath,
  listSources,
  type DetectionResult,
  type SourceRecord,
} from "../../lib/api";
import { PageFrame, RouteLink } from "../../components/Navigation";
import { EmptyState, Notice, SummaryItem } from "../../components/Presentation";
import { describeError, formatDate } from "../../lib/format";

// Detection always checks the paths saved in Settings — the same paths the
// background importer uses — so "detected" here means "will be imported".
const detectors: Record<string, () => Promise<DetectionResult>> = {
  "codex-session": () => detectCodexPath(null),
  "claude-code-session": () => detectClaudePath(null),
  "cc-switch": () => detectCcSwitchPath(null),
  cockpit: () => detectCockpitPath(null),
  "dsh-session": () => detectDshPath(null),
  opencode: () => detectOpenCodePath(null),
  "openai-official-quota": () => detectOfficialQuotaPath(),
};

type Detection =
  | { state: "running" }
  | { state: "done"; result: DetectionResult }
  | { state: "failed"; message: string };

function healthInfo(status: string | null): {
  className: string;
  label: string;
} {
  switch (status) {
    case "healthy":
      return { className: "health-badge ok", label: "正常" };
    case "error":
      return { className: "health-badge error", label: "错误" };
    case "not_found":
      return { className: "health-badge missing", label: "未找到" };
    case "stale":
      return { className: "health-badge stale", label: "已过期" };
    default:
      return { className: "health-badge", label: status || "Unavailable" };
  }
}

export function SourcesView() {
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [detections, setDetections] = useState<Record<string, Detection>>({});

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
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  async function detect(id: string) {
    const detector = detectors[id];
    if (!detector) return;
    setDetections((current) => ({ ...current, [id]: { state: "running" } }));
    try {
      const result = await detector();
      setDetections((current) => ({
        ...current,
        [id]: { state: "done", result },
      }));
    } catch (cause) {
      console.error(`检测 ${id} 失败`, cause);
      setDetections((current) => ({
        ...current,
        [id]: { state: "failed", message: describeError(cause) },
      }));
    }
  }

  const detectable = sources.filter((source) => detectors[source.id]);
  const anyRunning = Object.values(detections).some(
    (item) => item.state === "running",
  );

  return (
    <PageFrame
      actions={
        <>
          <RouteLink to="/settings">
            <span className="ghost-link">修改路径</span>
          </RouteLink>
          {detectable.length ? (
            <button
              className="primary-button"
              type="button"
              disabled={anyRunning}
              onClick={() =>
                void Promise.all(detectable.map((source) => detect(source.id)))
              }
            >
              {anyRunning ? "检测中…" : "全部检测"}
            </button>
          ) : null}
        </>
      }
      busy={anyRunning}
    >
      {error ? <Notice>{error}</Notice> : null}
      {sources.length ? (
        <section className="route-grid" aria-label="数据源状态">
          {sources.map((source, index) => {
            const health = healthInfo(source.health_status);
            const detection = detections[source.id];
            return (
              <article
                className="panel route-card source-card"
                key={source.id}
                data-glow
                data-health={source.health_status ?? "unknown"}
                style={{ "--i": index } as CSSProperties}
              >
                <div className="route-card-heading">
                  <div>
                    <p className="section-kicker">{source.adapter_type}</p>
                    <h2>{source.display_name}</h2>
                  </div>
                  <span
                    className={health.className}
                    title={source.health_status ?? undefined}
                  >
                    {health.label}
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
                </dl>
                {source.last_error ? (
                  <p className="card-error">{source.last_error}</p>
                ) : null}
                {detectors[source.id] ? (
                  <div className="card-foot">
                    <DetectionLine detection={detection} />
                    <button
                      className="quiet-button"
                      type="button"
                      disabled={detection?.state === "running"}
                      onClick={() => void detect(source.id)}
                      aria-label={`检测 ${source.display_name}`}
                    >
                      {detection?.state === "running" ? "检测中…" : "检测"}
                    </button>
                  </div>
                ) : null}
              </article>
            );
          })}
        </section>
      ) : isLoading ? (
        <section className="route-grid" aria-label="正在读取数据源">
          <div className="panel route-card skeleton-card" />
          <div className="panel route-card skeleton-card" />
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="尚未登记数据源"
            description="启动 Core 后会在此展示各 Adapter 的健康状态。"
          />
        </section>
      )}
    </PageFrame>
  );
}

function DetectionLine({ detection }: { detection: Detection | undefined }) {
  if (!detection) {
    return <span className="detection">使用设置中保存的路径</span>;
  }
  if (detection.state === "running") {
    return <span className="detection running">正在检测…</span>;
  }
  if (detection.state === "failed") {
    return (
      <span className="detection error">检测失败：{detection.message}</span>
    );
  }
  const { result } = detection;
  return (
    <span className={`detection ${result.detected ? "ok" : "error"}`}>
      {result.detected
        ? `已检测到${result.detected_version ? ` · ${result.detected_version}` : ""}`
        : `未检测到${result.message ? `：${result.message}` : ""}`}
    </span>
  );
}
