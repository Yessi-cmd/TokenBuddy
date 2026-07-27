import { useEffect, useState } from "react";

import {
  getAppSettings,
  isDesktopRuntime,
  pickDirectory,
  pickFile,
  updateAppSettings,
  type AppSettings,
} from "../../lib/api";
import { PageFrame } from "../../components/Navigation";
import { describeError } from "../../lib/format";

const defaultAppSettings: AppSettings = {
  codex_home: null,
  claude_home: null,
  cc_switch_db_path: null,
  cockpit_path: null,
  otel_port: null,
  auto_start: false,
  proxy_enabled: false,
  save_request_metadata: false,
  data_retention_days: null,
};

export function SettingsView() {
  const [settings, setSettings] = useState<AppSettings>(defaultAppSettings);
  const [status, setStatus] = useState("正在读取设置…");
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    let active = true;
    void getAppSettings()
      .then((nextSettings) => {
        if (active) {
          setSettings(nextSettings);
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

  // The picker only fills the field; nothing is written until the user saves,
  // and cancelling leaves the configured path untouched.
  async function browse(
    kind: "directory" | "file",
    title: string,
    field: "codex_home" | "claude_home" | "cc_switch_db_path" | "cockpit_path",
  ) {
    try {
      const picked =
        kind === "directory"
          ? await pickDirectory(title, settings[field])
          : await pickFile(title, settings[field]);
      if (picked) {
        setSettings((current) => ({ ...current, [field]: picked }));
        setStatus("路径已选择，尚未保存");
        setError(null);
      }
    } catch (cause) {
      console.error("打开选择器失败", cause);
      setError(`无法打开系统选择器：${describeError(cause)}`);
    }
  }

  async function handleSave() {
    setIsSaving(true);
    try {
      const nextSettings = await updateAppSettings({
        ...settings,
        codex_home: settings.codex_home?.trim() || null,
        claude_home: settings.claude_home?.trim() || null,
        cc_switch_db_path: settings.cc_switch_db_path?.trim() || null,
        cockpit_path: settings.cockpit_path?.trim() || null,
      });
      setSettings(nextSettings);
      setStatus("设置已保存");
      setError(null);
    } catch (cause) {
      console.error("保存设置失败", cause);
      setError(`设置保存失败：${describeError(cause)}`);
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <PageFrame
      eyebrow="Local configuration"
      title="设置"
      subtitle="Codex 与 Claude Code Session 路径由 Core 持久化并自动增量导入；其他 Adapter 保持 Unavailable。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section className="panel settings-panel" aria-label="应用设置">
        <div className="settings-heading">
          <div>
            <p className="section-kicker">采集路径</p>
            <h2>数据源路径</h2>
          </div>
          <span
            className="status-pill"
            data-state={error ? "warning" : "ready"}
          >
            {status}
          </span>
        </div>
        <div className="settings-grid">
          <SettingsField
            id="settings-codex-home"
            label="Codex Home"
            value={settings.codex_home}
            onChange={(value) =>
              setSettings({ ...settings, codex_home: value })
            }
            placeholder="留空使用系统默认路径"
            onBrowse={() =>
              browse("directory", "选择 Codex Home", "codex_home")
            }
          />
          <SettingsField
            id="settings-claude-home"
            label="Claude Home"
            value={settings.claude_home}
            onChange={(value) =>
              setSettings({ ...settings, claude_home: value })
            }
            placeholder="留空使用系统默认路径"
            onBrowse={() =>
              browse("directory", "选择 Claude Home", "claude_home")
            }
          />
          <SettingsField
            id="settings-cc-switch"
            label="CC Switch DB"
            value={settings.cc_switch_db_path}
            onChange={(value) =>
              setSettings({ ...settings, cc_switch_db_path: value })
            }
            placeholder="选择 cc-switch.db（只读）"
            onBrowse={() =>
              browse("file", "选择 CC Switch 数据库", "cc_switch_db_path")
            }
          />
          <SettingsField
            id="settings-cockpit"
            label="Cockpit 数据路径"
            value={settings.cockpit_path}
            onChange={(value) =>
              setSettings({ ...settings, cockpit_path: value })
            }
            placeholder="选择 codex_local_access_logs.sqlite（只读）"
            onBrowse={() =>
              browse("file", "选择 Cockpit 数据库", "cockpit_path")
            }
          />
        </div>
        <div className="settings-flags">
          <label>
            <input
              type="checkbox"
              checked={settings.auto_start}
              onChange={(event) =>
                setSettings({ ...settings, auto_start: event.target.checked })
              }
            />
            开机自动启动（修改后立即生效）
          </label>
          <label>
            <input
              type="checkbox"
              checked={settings.proxy_enabled}
              disabled
              readOnly
            />
            允许本地代理（Phase 7，当前关闭）
          </label>
        </div>
        <button
          className="primary-button"
          type="button"
          onClick={handleSave}
          disabled={isSaving}
        >
          {isSaving ? "保存中…" : "保存设置"}
        </button>
      </section>
    </PageFrame>
  );
}

function SettingsField({
  id,
  label,
  value,
  onChange,
  placeholder,
  onBrowse,
}: {
  id: string;
  label: string;
  value: string | null;
  onChange: (value: string) => void;
  placeholder: string;
  onBrowse?: () => void;
}) {
  return (
    <label className="settings-field" htmlFor={id}>
      <span>{label}</span>
      <div className="settings-field-input">
        <input
          id={id}
          value={value ?? ""}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
        />
        {onBrowse && isDesktopRuntime() ? (
          <button
            className="quiet-button"
            type="button"
            onClick={() => void onBrowse()}
            aria-label={`${label}：浏览`}
          >
            浏览…
          </button>
        ) : null}
      </div>
    </label>
  );
}
