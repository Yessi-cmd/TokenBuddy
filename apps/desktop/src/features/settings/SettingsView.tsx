import { useEffect, useRef, useState, type ReactNode } from "react";

import {
  detectCcSwitchPath,
  detectClaudePath,
  detectCockpitPath,
  detectCodexPath,
  detectDshPath,
  detectOpenCodePath,
  getAppSettings,
  isDesktopRuntime,
  pickDirectory,
  pickFile,
  updateAppSettings,
  type AppSettings,
  type DetectionResult,
} from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { Notice } from "../../components/Presentation";
import { describeError } from "../../lib/format";

const defaultAppSettings: AppSettings = {
  codex_home: null,
  claude_home: null,
  opencode_db_path: null,
  dsh_home: null,
  cc_switch_db_path: null,
  cockpit_path: null,
  otel_port: null,
  auto_start: false,
  proxy_enabled: false,
  save_request_metadata: false,
  data_retention_days: null,
};

type PathField =
  | "codex_home"
  | "claude_home"
  | "opencode_db_path"
  | "dsh_home"
  | "cc_switch_db_path"
  | "cockpit_path";

const pathFields: {
  field: PathField;
  id: string;
  label: string;
  kind: "directory" | "file";
  pickerTitle: string;
  placeholder: string;
  detect: (path: string | null) => Promise<DetectionResult>;
}[] = [
  {
    field: "codex_home",
    id: "settings-codex-home",
    label: "Codex Home",
    kind: "directory",
    pickerTitle: "选择 Codex Home",
    placeholder: "留空使用系统默认路径",
    detect: detectCodexPath,
  },
  {
    field: "claude_home",
    id: "settings-claude-home",
    label: "Claude Home",
    kind: "directory",
    pickerTitle: "选择 Claude Home",
    placeholder: "留空使用系统默认路径",
    detect: detectClaudePath,
  },
  {
    field: "opencode_db_path",
    id: "settings-opencode",
    label: "OpenCode 数据库",
    kind: "file",
    pickerTitle: "选择 OpenCode 数据库",
    placeholder: "选择 opencode.db（只读）",
    detect: detectOpenCodePath,
  },
  {
    field: "dsh_home",
    id: "settings-dsh-home",
    label: "DeepSeek Harness Home",
    kind: "directory",
    pickerTitle: "选择 DeepSeek Harness Home",
    placeholder: "留空使用 ~/.dsh（或 $DSH_HOME）",
    detect: detectDshPath,
  },
  {
    field: "cc_switch_db_path",
    id: "settings-cc-switch",
    label: "CC Switch DB",
    kind: "file",
    pickerTitle: "选择 CC Switch 数据库",
    placeholder: "选择 cc-switch.db（只读）",
    detect: detectCcSwitchPath,
  },
  {
    field: "cockpit_path",
    id: "settings-cockpit",
    label: "Cockpit 数据路径",
    kind: "file",
    pickerTitle: "选择 Cockpit 数据库",
    placeholder: "选择 codex_local_access_logs.sqlite（只读）",
    detect: detectCockpitPath,
  },
];

type Detection =
  | { state: "running" }
  | { state: "done"; result: DetectionResult }
  | { state: "failed"; message: string };

function parsePort(raw: string): number | null {
  const value = raw.trim() ? Number(raw.trim()) : null;
  return value !== null &&
    Number.isInteger(value) &&
    value >= 1024 &&
    value <= 65535
    ? value
    : null;
}

function parseDays(raw: string): number | null {
  const value = raw.trim() ? Number(raw.trim()) : null;
  return value !== null && Number.isInteger(value) && value >= 1 ? value : null;
}

function normalize(settings: AppSettings, port: string, days: string) {
  return {
    ...settings,
    codex_home: settings.codex_home?.trim() || null,
    claude_home: settings.claude_home?.trim() || null,
    opencode_db_path: settings.opencode_db_path?.trim() || null,
    dsh_home: settings.dsh_home?.trim() || null,
    cc_switch_db_path: settings.cc_switch_db_path?.trim() || null,
    cockpit_path: settings.cockpit_path?.trim() || null,
    otel_port: parsePort(port),
    data_retention_days: parseDays(days),
  };
}

export function SettingsView() {
  const [settings, setSettings] = useState<AppSettings>(defaultAppSettings);
  const [saved, setSaved] = useState<AppSettings | null>(null);
  // Numeric fields keep the raw text while typing. Validating per keystroke
  // used to reset the field the moment a partial value (like "8" on the way to
  // "8080") was out of range, which made the port impossible to type.
  const [portDraft, setPortDraft] = useState("");
  const [daysDraft, setDaysDraft] = useState("");
  const [status, setStatus] = useState("正在读取设置…");
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [detections, setDetections] = useState<
    Partial<Record<PathField, Detection>>
  >({});
  const saveRef = useRef<() => void>(() => {});

  useEffect(() => {
    let active = true;
    void getAppSettings()
      .then((nextSettings) => {
        if (active) {
          setSettings(nextSettings);
          setSaved(nextSettings);
          setPortDraft(nextSettings.otel_port?.toString() ?? "");
          setDaysDraft(nextSettings.data_retention_days?.toString() ?? "");
          setStatus("设置已加载");
          setError(null);
        }
      })
      .catch(() => {
        if (active) {
          setStatus("设置不可用");
          setError("无法读取 Core 设置。");
        }
      });
    return () => {
      active = false;
    };
  }, []);

  const pending = normalize(settings, portDraft, daysDraft);
  // Saving before a successful read would overwrite the real configuration
  // with blank defaults, so the form stays read-only until it has loaded.
  const loaded = saved !== null;
  // Compare the raw numeric drafts too: an invalid draft parses to the same
  // null as an empty field, but the user has still changed something.
  const dirty =
    loaded &&
    (JSON.stringify(pending) !== JSON.stringify(saved) ||
      portDraft.trim() !== (saved.otel_port?.toString() ?? "") ||
      daysDraft.trim() !== (saved.data_retention_days?.toString() ?? ""));
  const portInvalid = portDraft.trim() !== "" && parsePort(portDraft) === null;
  const daysInvalid = daysDraft.trim() !== "" && parseDays(daysDraft) === null;

  function update<K extends keyof AppSettings>(key: K, value: AppSettings[K]) {
    setSettings((current) => ({ ...current, [key]: value }));
  }

  // The picker only fills the field; nothing is written until the user saves,
  // and cancelling leaves the configured path untouched.
  async function browse(
    kind: "directory" | "file",
    title: string,
    field: PathField,
  ) {
    try {
      const picked =
        kind === "directory"
          ? await pickDirectory(title, settings[field])
          : await pickFile(title, settings[field]);
      if (picked) {
        update(field, picked);
        setError(null);
      }
    } catch (cause) {
      console.error("打开选择器失败", cause);
      setError(`无法打开系统选择器：${describeError(cause)}`);
    }
  }

  // Checks the path as typed, before saving, so a wrong path is caught here
  // rather than discovered later as an empty dashboard.
  async function detect(
    field: PathField,
    run: (path: string | null) => Promise<DetectionResult>,
  ) {
    setDetections((current) => ({ ...current, [field]: { state: "running" } }));
    try {
      const result = await run(settings[field]?.trim() || null);
      setDetections((current) => ({
        ...current,
        [field]: { state: "done", result },
      }));
    } catch (cause) {
      console.error(`检测 ${field} 失败`, cause);
      setDetections((current) => ({
        ...current,
        [field]: { state: "failed", message: describeError(cause) },
      }));
    }
  }

  async function handleSave() {
    if (!loaded) return;
    setIsSaving(true);
    try {
      const nextSettings = await updateAppSettings(pending);
      setSettings(nextSettings);
      setSaved(nextSettings);
      setPortDraft(nextSettings.otel_port?.toString() ?? "");
      setDaysDraft(nextSettings.data_retention_days?.toString() ?? "");
      setStatus("设置已保存");
      setError(null);
    } catch (cause) {
      console.error("保存设置失败", cause);
      setError(`设置保存失败：${describeError(cause)}`);
    } finally {
      setIsSaving(false);
    }
  }

  function handleDiscard() {
    if (!saved) return;
    setSettings(saved);
    setPortDraft(saved.otel_port?.toString() ?? "");
    setDaysDraft(saved.data_retention_days?.toString() ?? "");
    setDetections({});
  }

  saveRef.current = () => {
    if (dirty && !isSaving) void handleSave();
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        saveRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const desktop = isDesktopRuntime();

  return (
    <PageFrame>
      {error ? <Notice>{error}</Notice> : null}
      <section className="panel settings-panel" aria-label="应用设置">
        <SettingsSection
          kicker="采集路径"
          title="数据源路径"
          description="全部以只读方式访问；留空时使用系统默认路径。点“检测”可在保存前确认路径有效。"
        >
          <div className="settings-grid">
            {pathFields.map((item) => (
              <SettingsField
                key={item.field}
                id={item.id}
                label={item.label}
                value={settings[item.field]}
                onChange={(value) => {
                  update(item.field, value);
                  setDetections((current) => ({
                    ...current,
                    [item.field]: undefined,
                  }));
                }}
                placeholder={item.placeholder}
                onBrowse={
                  desktop
                    ? () => browse(item.kind, item.pickerTitle, item.field)
                    : undefined
                }
                onDetect={() => void detect(item.field, item.detect)}
                detection={detections[item.field]}
              />
            ))}
          </div>
        </SettingsSection>

        <SettingsSection
          kicker="采集与存储"
          title="接收与保留"
          description="OTLP 只监听 127.0.0.1；数据保留周期作用于用量事件、会话和额度快照。"
        >
          <div className="settings-grid">
            <label className="settings-field" htmlFor="settings-otel-port">
              <span>OTLP HTTP 端口</span>
              <input
                id="settings-otel-port"
                type="number"
                min={1024}
                max={65535}
                inputMode="numeric"
                value={portDraft}
                aria-invalid={portInvalid || undefined}
                onChange={(event) => setPortDraft(event.target.value)}
                placeholder="留空关闭（仅 127.0.0.1）"
                title="TokenBuddy 只监听 127.0.0.1，不接受局域网连接；端口被占用时主采集仍会继续。"
              />
              <small
                className={`settings-help ${portInvalid ? "is-invalid" : ""}`}
              >
                {portInvalid
                  ? "端口需为 1024–65535 的整数；无效值保存时按留空（关闭）处理。"
                  : "可接收 OTLP/HTTP traces；默认关闭，不需要 Collector。"}
              </small>
            </label>
            <label className="settings-field" htmlFor="settings-retention-days">
              <span>数据保留周期（天）</span>
              <input
                id="settings-retention-days"
                type="number"
                min={1}
                inputMode="numeric"
                value={daysDraft}
                aria-invalid={daysInvalid || undefined}
                onChange={(event) => setDaysDraft(event.target.value)}
                placeholder="留空永久保留"
              />
              <small
                className={`settings-help ${daysInvalid ? "is-invalid" : ""}`}
              >
                {daysInvalid
                  ? "需为正整数；无效值保存时按永久保留处理。"
                  : "超过该天数的记录会在下一次导入时清理；留空或 0 表示永久保留。"}
              </small>
            </label>
          </div>
        </SettingsSection>

        <SettingsSection kicker="启动与隐私" title="后台运行">
          <div className="settings-toggles">
            <Toggle
              id="settings-auto-start"
              checked={settings.auto_start}
              onChange={(checked) => update("auto_start", checked)}
              title="开机自启"
              description="登录系统后在后台启动 TokenBuddy，不自动弹出完整面板；保存设置后生效。"
            />
            <Toggle
              id="settings-save-metadata"
              checked={settings.save_request_metadata}
              onChange={(checked) => update("save_request_metadata", checked)}
              title="保存脱敏 usage 元数据"
              description="只保存脱敏 usage 字段，不保存 Prompt、Completion 或源代码；关闭后会清除已保存的原始 usage 元数据。默认关闭。"
            />
            <Toggle
              id="settings-proxy"
              checked={settings.proxy_enabled}
              disabled
              onChange={() => {}}
              title="允许本地代理"
              description="Phase 7 功能，当前关闭；代理永远不是启动或统计的前提。"
            />
          </div>
        </SettingsSection>

        <div className="settings-savebar" data-dirty={dirty || undefined}>
          <span
            className="status-pill"
            data-state={error ? "warning" : dirty ? "dirty" : "ready"}
          >
            <span className="status-dot" aria-hidden="true" />
            {dirty ? "有未保存的修改" : status}
          </span>
          <span className="savebar-hint">Ctrl / ⌘ + S</span>
          {dirty ? (
            <button
              className="ghost-button"
              type="button"
              onClick={handleDiscard}
              disabled={isSaving}
            >
              放弃修改
            </button>
          ) : null}
          <button
            className="primary-button"
            type="button"
            onClick={handleSave}
            disabled={isSaving || !loaded}
          >
            {isSaving ? "保存中…" : "保存设置"}
          </button>
        </div>
      </section>
    </PageFrame>
  );
}

function SettingsSection({
  kicker,
  title,
  description,
  children,
}: {
  kicker: string;
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-section">
      <div className="settings-subheading">
        <p className="section-kicker">{kicker}</p>
        <h3>{title}</h3>
        {description ? <p className="settings-help">{description}</p> : null}
      </div>
      {children}
    </div>
  );
}

function Toggle({
  id,
  checked,
  onChange,
  title,
  description,
  disabled = false,
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  title: string;
  description: string;
  disabled?: boolean;
}) {
  return (
    <label
      className="settings-toggle-option"
      htmlFor={id}
      data-disabled={disabled || undefined}
    >
      <input
        id={id}
        className="settings-toggle-input"
        type="checkbox"
        checked={checked}
        disabled={disabled}
        aria-describedby={`${id}-help`}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="settings-toggle-control" aria-hidden="true" />
      <span className="settings-toggle-copy">
        <strong>{title}</strong>
        <small id={`${id}-help`}>{description}</small>
      </span>
    </label>
  );
}

function SettingsField({
  id,
  label,
  value,
  onChange,
  placeholder,
  onBrowse,
  onDetect,
  detection,
}: {
  id: string;
  label: string;
  value: string | null;
  onChange: (value: string) => void;
  placeholder: string;
  onBrowse?: () => void;
  onDetect: () => void;
  detection: Detection | undefined;
}) {
  return (
    <div className="settings-field">
      <label htmlFor={id}>{label}</label>
      <div className="settings-field-input">
        <input
          id={id}
          value={value ?? ""}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          spellCheck={false}
        />
        {onBrowse ? (
          <button
            className="quiet-button"
            type="button"
            onClick={() => void onBrowse()}
            aria-label={`${label}：浏览`}
          >
            浏览…
          </button>
        ) : null}
        <button
          className="quiet-button"
          type="button"
          onClick={onDetect}
          disabled={detection?.state === "running"}
          aria-label={`${label}：检测`}
        >
          {detection?.state === "running" ? "检测中…" : "检测"}
        </button>
      </div>
      {detection && detection.state !== "running" ? (
        <small
          className={`detection-line ${
            detection.state === "done" && detection.result.detected
              ? "ok"
              : "error"
          }`}
          role="status"
        >
          {detection.state === "failed"
            ? `检测失败：${detection.message}`
            : detection.result.detected
              ? `已检测到 · ${detection.result.path_or_endpoint ?? "默认路径"}${
                  detection.result.detected_version
                    ? ` · ${detection.result.detected_version}`
                    : ""
                }`
              : `未检测到${detection.result.message ? `：${detection.result.message}` : ""}`}
        </small>
      ) : null}
    </div>
  );
}
