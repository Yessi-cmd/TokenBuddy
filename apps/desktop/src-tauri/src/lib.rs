//! The TokenBuddy desktop shell.
//!
//! Owns the tray, the windows, the `#[tauri::command]` layer, and the optional
//! loopback HTTP server. It holds an `Arc<Core>` and never touches SQLite or a
//! source file itself, so every surface it exposes reads the same data through
//! the same aggregation.
#![warn(missing_docs)]

mod web;

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration as StdDuration,
};

use tauri::{
    App, AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Rect, Runtime, State,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
    menu::{MenuBuilder, MenuEvent},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Effect, EffectState, EffectsBuilder},
};
use tokenbuddy_cc_switch::{CcSwitchAdapter, default_cc_switch_db};
use tokenbuddy_claude_session::{ClaudeSessionAdapter, default_claude_home};
use tokenbuddy_cockpit::{CockpitAdapter, default_cockpit_db};
use tokenbuddy_codex_session::{CodexSessionAdapter, default_codex_home};
use tokenbuddy_core::{Core, CoreConfig, CoreError, ImportReport};
use tokenbuddy_domain::{
    AppSettings, DashboardSummary, DetectionResult, ExportResult, QuickSummary, SessionDetail,
    SessionPage, UsageFilters,
};
use web::{AutostartCallback, LocalWebApiStatus, LocalWebServer};

const QUICK_WINDOW_MIN_HEIGHT: f64 = 120.0;
const QUICK_WINDOW_MARGIN: i32 = 8;

struct AppState {
    core: Arc<Core>,
    web_server: Mutex<Option<LocalWebServer>>,
    quitting: AtomicBool,
    quick_hide_generation: AtomicU64,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("你好，{name}！TokenBuddy 的前后端通信正常。")
}

#[tauri::command]
fn get_quick_summary(
    state: State<'_, AppState>,
) -> Result<tokenbuddy_domain::QuickSummary, String> {
    state.core.quick_summary().map_err(core_error)
}

#[tauri::command]
fn get_dashboard_summary(
    state: State<'_, AppState>,
    filters: Option<UsageFilters>,
) -> Result<DashboardSummary, String> {
    state
        .core
        .dashboard_summary_filtered(filters.unwrap_or_default())
        .map_err(core_error)
}

#[tauri::command]
fn get_model_breakdown(
    state: State<'_, AppState>,
    filters: Option<UsageFilters>,
) -> Result<Vec<tokenbuddy_domain::ModelUsage>, String> {
    state
        .core
        .model_breakdown(filters.unwrap_or_default())
        .map_err(core_error)
}

#[tauri::command]
fn list_sessions(
    state: State<'_, AppState>,
    filters: Option<UsageFilters>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<SessionPage, String> {
    state
        .core
        .list_sessions(
            &filters.unwrap_or_default(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )
        .map_err(core_error)
}

#[tauri::command]
fn get_session_detail(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<SessionDetail>, String> {
    state
        .core
        .get_session_detail(&session_id)
        .map_err(core_error)
}

#[tauri::command]
fn list_usage_events(
    state: State<'_, AppState>,
    session_id: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<tokenbuddy_domain::UsageEventPage, String> {
    state
        .core
        .list_usage_events(
            session_id.as_deref(),
            limit.unwrap_or(100),
            offset.unwrap_or(0),
        )
        .map_err(core_error)
}

#[tauri::command]
fn list_sources(
    state: State<'_, AppState>,
) -> Result<Vec<tokenbuddy_domain::SourceRecord>, String> {
    state.core.list_sources().map_err(core_error)
}

#[tauri::command]
fn list_providers(
    state: State<'_, AppState>,
) -> Result<Vec<tokenbuddy_domain::ProviderSummary>, String> {
    state.core.list_providers().map_err(core_error)
}

#[tauri::command]
fn list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<tokenbuddy_domain::AccountSummary>, String> {
    state.core.list_accounts().map_err(core_error)
}

/// Native directory picker for the Settings page. `None` means the user
/// cancelled — never an error, and never a silent change to the stored path.
///
/// Only the desktop shell can show a system dialog; the loopback web panel
/// keeps its text field, which is why this is a Tauri command with no `/api`
/// counterpart.
#[tauri::command]
async fn pick_directory(
    app: AppHandle,
    title: Option<String>,
    start_at: Option<String>,
) -> Result<Option<String>, String> {
    pick_path(app, title, start_at, PickKind::Directory).await
}

#[tauri::command]
async fn pick_file(
    app: AppHandle,
    title: Option<String>,
    start_at: Option<String>,
) -> Result<Option<String>, String> {
    pick_path(app, title, start_at, PickKind::File).await
}

#[tauri::command]
fn list_quota_snapshots(
    state: State<'_, AppState>,
    account_id: Option<String>,
    limit: Option<u64>,
) -> Result<Vec<tokenbuddy_domain::QuotaSnapshot>, String> {
    state
        .core
        .list_quota_snapshots(account_id.as_deref(), limit.unwrap_or(100))
        .map_err(core_error)
}

#[tauri::command]
fn refresh_official_quota(state: State<'_, AppState>) -> Result<ImportReport, String> {
    state.core.refresh_official_quota().map_err(core_error)
}

#[tauri::command]
fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.core.get_app_settings().map_err(core_error)
}

#[tauri::command]
fn update_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let previous = state.core.get_app_settings().map_err(core_error)?;
    state
        .core
        .update_app_settings(settings.clone())
        .map_err(core_error)?;
    if previous.auto_start != settings.auto_start {
        sync_autostart(&app, settings.auto_start)?;
    }
    state.core.get_app_settings().map_err(core_error)
}

#[tauri::command]
fn export_usage(
    state: State<'_, AppState>,
    format: String,
    filters: Option<UsageFilters>,
) -> Result<ExportResult, String> {
    state
        .core
        .export_usage(&format, &filters.unwrap_or_default())
        .map_err(core_error)
}

#[tauri::command]
fn save_export(
    app: AppHandle,
    state: State<'_, AppState>,
    format: String,
    filters: Option<UsageFilters>,
) -> Result<String, String> {
    // WKWebView cannot reliably trigger a blob download, so the desktop app
    // writes the export to disk itself and returns the saved path.
    let export = state
        .core
        .export_usage(&format, &filters.unwrap_or_default())
        .map_err(core_error)?;
    let directory = app
        .path()
        .download_dir()
        .or_else(|_| app.path().document_dir())
        .map_err(|error| format!("无法定位保存目录：{error}"))?;
    std::fs::create_dir_all(&directory).map_err(|error| format!("无法创建保存目录：{error}"))?;
    let path = directory.join(&export.filename);
    std::fs::write(&path, export.content).map_err(|error| format!("无法写入导出文件：{error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn show_main_window(app: AppHandle) -> Result<(), String> {
    show_window(&app, "main");
    Ok(())
}

/// Resize the quick panel after the webview has measured its content, then
/// re-anchor it to the tray. Windows places the panel above the taskbar, so a
/// height-only resize would otherwise leave a visible gap below the panel.
#[tauri::command]
fn fit_quick_window_to_content(app: AppHandle, height: f64) -> Result<(), String> {
    if !height.is_finite() || height <= 0.0 {
        return Err("快速面板高度无效".to_owned());
    }

    let window = app
        .get_webview_window("quick")
        .ok_or_else(|| "快速面板窗口尚未创建".to_owned())?;
    let scale_factor = window
        .scale_factor()
        .map_err(|error| format!("无法读取快速面板缩放比例：{error}"))?;
    let current_size = window
        .inner_size()
        .map_err(|error| format!("无法读取快速面板尺寸：{error}"))?;
    let current_outer_size = window.outer_size().unwrap_or(current_size);
    let width = current_size.to_logical(scale_factor).width;
    let anchor = tray_rect(&app);
    let target_monitor = anchor
        .map(|anchor| anchor.position.to_physical::<i32>(scale_factor))
        .and_then(|position| {
            app.monitor_from_point(f64::from(position.x), f64::from(position.y))
                .ok()
                .flatten()
        })
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let maximum_height = target_monitor.as_ref().and_then(|monitor| {
        max_quick_inner_height(
            monitor.work_area().size.height,
            current_size.height,
            current_outer_size.height,
            scale_factor,
            monitor.scale_factor(),
        )
    });
    let height = fitted_quick_height(height, maximum_height);

    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| format!("无法调整快速面板尺寸：{error}"))?;
    if let Some(anchor) = anchor {
        position_quick_window(&app, &window, anchor);
    }
    Ok(())
}

fn fitted_quick_height(requested_height: f64, maximum_height: Option<f64>) -> f64 {
    let requested_height = requested_height.ceil().max(QUICK_WINDOW_MIN_HEIGHT);
    maximum_height
        .filter(|height| height.is_finite() && *height > 0.0)
        .map_or(requested_height, |maximum_height| {
            requested_height.min(maximum_height.max(QUICK_WINDOW_MIN_HEIGHT))
        })
}

fn max_quick_inner_height(
    work_area_height: u32,
    current_inner_height: u32,
    current_outer_height: u32,
    current_scale: f64,
    target_scale: f64,
) -> Option<f64> {
    if !valid_scale_factor(current_scale) || !valid_scale_factor(target_scale) {
        return None;
    }

    let frame_height =
        f64::from(current_outer_height.saturating_sub(current_inner_height)) / current_scale;
    let work_area_height = f64::from(work_area_height) / target_scale;
    let margin_height = f64::from(QUICK_WINDOW_MARGIN.saturating_mul(2)) / target_scale;
    let maximum_height = (work_area_height - frame_height - margin_height).floor();
    (maximum_height.is_finite() && maximum_height > 0.0).then_some(maximum_height)
}

fn valid_scale_factor(scale_factor: f64) -> bool {
    scale_factor.is_finite() && scale_factor > 0.0
}

fn physical_size_at_scale(
    size: PhysicalSize<u32>,
    current_scale: f64,
    target_scale: f64,
) -> PhysicalSize<u32> {
    if !valid_scale_factor(current_scale) || !valid_scale_factor(target_scale) {
        return size;
    }
    size.to_logical::<f64>(current_scale)
        .to_physical::<u32>(target_scale)
}

#[tauri::command]
fn detect_codex_path(
    state: State<'_, AppState>,
    codex_home: Option<String>,
) -> Result<DetectionResult, String> {
    if let Some(home) = normalized_path(codex_home) {
        return CodexSessionAdapter::new(home)
            .detect_sync()
            .map_err(|error| error.to_string());
    }
    state.core.detect_codex_path().map_err(core_error)
}

#[tauri::command]
fn detect_official_quota_path(state: State<'_, AppState>) -> Result<DetectionResult, String> {
    state.core.detect_official_quota_path().map_err(core_error)
}

#[tauri::command]
fn rescan_codex(
    state: State<'_, AppState>,
    codex_home: Option<String>,
) -> Result<ImportReport, String> {
    state
        .core
        .rescan_codex(normalized_path(codex_home))
        .map_err(core_error)
}

#[tauri::command]
fn detect_claude_path(
    state: State<'_, AppState>,
    claude_home: Option<String>,
) -> Result<DetectionResult, String> {
    if let Some(home) = normalized_path(claude_home) {
        return ClaudeSessionAdapter::new(home)
            .detect_sync()
            .map_err(|error| error.to_string());
    }
    state.core.detect_claude_path().map_err(core_error)
}

#[tauri::command]
fn rescan_claude(
    state: State<'_, AppState>,
    claude_home: Option<String>,
) -> Result<ImportReport, String> {
    state
        .core
        .rescan_claude(normalized_path(claude_home))
        .map_err(core_error)
}

#[tauri::command]
fn detect_cc_switch_path(
    state: State<'_, AppState>,
    cc_switch_db: Option<String>,
) -> Result<DetectionResult, String> {
    if let Some(path) = normalized_path(cc_switch_db) {
        return CcSwitchAdapter::new(path)
            .detect_sync()
            .map_err(|error| error.to_string());
    }
    state.core.detect_cc_switch_path().map_err(core_error)
}

#[tauri::command]
fn rescan_cc_switch(
    state: State<'_, AppState>,
    cc_switch_db: Option<String>,
) -> Result<ImportReport, String> {
    state
        .core
        .rescan_cc_switch(normalized_path(cc_switch_db))
        .map_err(core_error)
}

#[tauri::command]
fn detect_cockpit_path(
    state: State<'_, AppState>,
    cockpit_db: Option<String>,
) -> Result<DetectionResult, String> {
    if let Some(path) = normalized_path(cockpit_db) {
        return CockpitAdapter::new(path)
            .detect_sync()
            .map_err(|error| error.to_string());
    }
    state.core.detect_cockpit_path().map_err(core_error)
}

#[tauri::command]
fn rescan_cockpit(
    state: State<'_, AppState>,
    cockpit_db: Option<String>,
) -> Result<ImportReport, String> {
    state
        .core
        .rescan_cockpit(normalized_path(cockpit_db))
        .map_err(core_error)
}

#[tauri::command]
fn start_local_web_api(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LocalWebApiStatus, String> {
    let mut web_server = state
        .web_server
        .lock()
        .map_err(|_| "本地网页服务锁已损坏，请重启 TokenBuddy".to_owned())?;
    if let Some(server) = web_server.as_ref() {
        return Ok(server.status());
    }

    let static_root = resolve_web_root(&app);
    let server = start_local_web_server(&app, Arc::clone(&state.core), static_root)
        .map_err(|error| format!("无法启动本地网页服务：{error}"))?;
    let status = server.status();
    *web_server = Some(server);
    Ok(status)
}

#[tauri::command]
fn stop_local_web_api(state: State<'_, AppState>) -> Result<LocalWebApiStatus, String> {
    let mut web_server = state
        .web_server
        .lock()
        .map_err(|_| "本地网页服务锁已损坏，请重启 TokenBuddy".to_owned())?;
    web_server.take();
    Ok(LocalWebApiStatus {
        running: false,
        url: None,
        loopback_urls: Vec::new(),
    })
}

#[tauri::command]
fn get_local_web_api_status(state: State<'_, AppState>) -> Result<LocalWebApiStatus, String> {
    let web_server = state
        .web_server
        .lock()
        .map_err(|_| "本地网页服务锁已损坏，请重启 TokenBuddy".to_owned())?;
    Ok(web_server.as_ref().map_or(
        LocalWebApiStatus {
            running: false,
            url: None,
            loopback_urls: Vec::new(),
        },
        LocalWebServer::status,
    ))
}

#[tauri::command]
fn open_local_web_api(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LocalWebApiStatus, String> {
    let status = start_local_web_api(app.clone(), state)?;
    if let Some(url) = status.url.as_deref() {
        open_url(url)?;
    }
    Ok(status)
}

#[tauri::command]
fn quit_tokenbuddy(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.quitting.store(true, Ordering::SeqCst);
    state.core.shutdown().map_err(core_error)?;
    app.exit(0);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKind {
    Directory,
    File,
}

async fn pick_path(
    app: AppHandle,
    title: Option<String>,
    start_at: Option<String>,
    kind: PickKind,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::{DialogExt, FilePath};

    let mut builder = app.dialog().file();
    if let Some(title) = title {
        builder = builder.set_title(title);
    }
    if let Some(directory) = start_directory(start_at) {
        builder = builder.set_directory(directory);
    }
    if kind == PickKind::File {
        builder = builder
            .add_filter("SQLite 数据库", &["db", "sqlite", "sqlite3"])
            .add_filter("所有文件", &["*"]);
    }

    // The picker answers on the UI thread through a callback. Hand the result
    // back over a one-slot channel with a non-blocking send, so neither the UI
    // thread nor this task ever blocks the other.
    let (sender, mut receiver) = tauri::async_runtime::channel::<Option<FilePath>>(1);
    let deliver = move |picked: Option<FilePath>| {
        let _ = sender.try_send(picked);
    };
    match kind {
        PickKind::Directory => builder.pick_folder(deliver),
        PickKind::File => builder.pick_file(deliver),
    }

    let picked = receiver
        .recv()
        .await
        .ok_or_else(|| "文件选择器没有返回结果".to_owned())?;
    Ok(picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned()))
}

/// Open the picker where the user already pointed TokenBuddy: the configured
/// directory, or the parent of the configured file. A path that no longer exists
/// is ignored so the dialog falls back to the system default instead of failing.
fn start_directory(start_at: Option<String>) -> Option<PathBuf> {
    let path = normalized_path(start_at)?;
    if path.is_dir() {
        return Some(path);
    }
    path.parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
}

fn normalized_path(path: Option<String>) -> Option<PathBuf> {
    path.filter(|value| !value.trim().is_empty())
        .map(|value| PathBuf::from(value.trim()))
}

fn resolve_web_root<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Some(path) = std::env::var_os("TOKENBUDDY_WEB_ROOT") {
        return PathBuf::from(path);
    }
    let development_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if development_root.join("index.html").is_file() {
        return development_root;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        if resource_dir.join("index.html").is_file() {
            return resource_dir;
        }
        return resource_dir.join("dist");
    }
    development_root
}

fn start_local_web_server<R: Runtime>(
    app: &AppHandle<R>,
    core: Arc<Core>,
    static_root: PathBuf,
) -> std::io::Result<LocalWebServer> {
    let callback_app = app.clone();
    let autostart: AutostartCallback =
        Arc::new(move |enabled| sync_autostart(&callback_app, enabled));
    LocalWebServer::start_with_autostart(core, static_root, Some(autostart))
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).status();
    result
        .map(|_| ())
        .map_err(|error| format!("无法打开浏览器：{error}"))
}

fn core_error(error: CoreError) -> String {
    error.to_string()
}

#[cfg(windows)]
const WINDOWS_RUN_REGISTRY_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

#[cfg(windows)]
const WINDOWS_STARTUP_APPROVED_REGISTRY_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

#[cfg(windows)]
const WINDOWS_STARTUP_APPROVED_ENABLED_VALUE: [u8; 12] = [
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[cfg(windows)]
fn sync_autostart<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), String> {
    use std::io::ErrorKind;

    use winreg::{
        RegKey, RegValue,
        enums::{HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY},
    };

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = if enabled {
        current_user
            .create_subkey(WINDOWS_RUN_REGISTRY_KEY)
            .map(|(key, _)| key)
            .map_err(|error| format!("无法打开 Windows 自启动注册表项：{error}"))?
    } else {
        match current_user.open_subkey_with_flags(WINDOWS_RUN_REGISTRY_KEY, KEY_SET_VALUE) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("无法打开 Windows 自启动注册表项：{error}")),
        }
    };
    let value_name = &app.package_info().name;

    if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| format!("无法定位 TokenBuddy 可执行文件：{error}"))?;
        let command_line = windows_autostart_command_line(&executable);
        run_key
            .set_value(value_name, &command_line)
            .map_err(|error| format!("无法写入 Windows 自启动项：{error}"))?;

        // Windows may keep a separate StartupApproved value after a user has
        // disabled an entry in Task Manager. Mark the entry enabled when we
        // explicitly enable it from TokenBuddy.
        if let Ok(startup_approved_key) = current_user
            .open_subkey_with_flags(WINDOWS_STARTUP_APPROVED_REGISTRY_KEY, KEY_SET_VALUE)
        {
            startup_approved_key
                .set_raw_value(
                    value_name,
                    &RegValue {
                        vtype: REG_BINARY,
                        bytes: WINDOWS_STARTUP_APPROVED_ENABLED_VALUE.to_vec(),
                    },
                )
                .map_err(|error| format!("无法更新 Windows 自启动状态：{error}"))?;
        }

        Ok(())
    } else {
        match run_key.delete_value(value_name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法关闭开机启动：{error}")),
        }
    }
}

#[cfg(not(windows))]
fn sync_autostart<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|error| format!("无法启用开机启动：{error}"))
    } else {
        manager
            .disable()
            .map_err(|error| format!("无法关闭开机启动：{error}"))
    }
}

#[cfg(any(windows, test))]
fn windows_autostart_command_line(executable: &Path) -> String {
    format!("\"{}\"", executable.display())
}

fn show_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(window) = get_or_create_window(app, label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn get_or_create_window<R: Runtime>(app: &AppHandle<R>, label: &str) -> Option<WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(label) {
        return Some(window);
    }

    let result = match label {
        "main" => WebviewWindowBuilder::new(app, "main", WebviewUrl::App("/dashboard".into()))
            .title("TokenBuddy")
            .inner_size(1100.0, 720.0)
            .min_inner_size(860.0, 600.0)
            .resizable(true)
            .visible(false)
            .build(),
        "quick" => quick_window_builder(app).build(),
        _ => return None,
    };
    result.ok()
}

fn quick_window_builder<R: Runtime>(
    app: &AppHandle<R>,
) -> WebviewWindowBuilder<'_, R, AppHandle<R>> {
    let builder = WebviewWindowBuilder::new(app, "quick", WebviewUrl::App("/quick".into()))
        .title("TokenBuddy QuickSummary")
        // A first-paint size only: the webview measures its own content and
        // shrinks the window to fit (see `fitQuickWindowToContent`). The lower
        // bound must therefore be small enough for a summary that has no quota
        // window and no warning, or the popover keeps dead space below its last
        // row.
        .inner_size(320.0, 420.0)
        .min_inner_size(300.0, 120.0)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .visible(false);
    // A translucent system material (NSVisualEffectView on macOS, Acrylic on
    // Windows) makes the panel read as a native menu-bar popover rather than an
    // opaque web page. Unsupported platforms fall back to the CSS surface.
    let effect = if cfg!(target_os = "windows") {
        Effect::Acrylic
    } else {
        Effect::Popover
    };
    builder.effects(
        EffectsBuilder::new()
            .effect(effect)
            .state(EffectState::Active)
            .radius(12.0)
            .build(),
    )
}

fn tray_rect<R: Runtime>(app: &AppHandle<R>) -> Option<Rect> {
    app.tray_by_id("main")
        .and_then(|tray| tray.rect().ok().flatten())
}

fn show_quick_window<R: Runtime>(app: &AppHandle<R>) {
    show_quick_window_at(app, tray_rect(app));
}

fn show_quick_window_at<R: Runtime>(app: &AppHandle<R>, anchor: Option<Rect>) {
    cancel_scheduled_quick_hide(app);
    if let Some(window) = get_or_create_window(app, "quick") {
        if let Some(anchor) = anchor {
            position_quick_window(app, &window, anchor);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.hide();
    }
}

fn toggle_quick_window<R: Runtime>(app: &AppHandle<R>, anchor: Rect) {
    cancel_scheduled_quick_hide(app);
    if let Some(window) = get_or_create_window(app, "quick") {
        if next_window_visible(window.is_visible().unwrap_or(false)) {
            show_quick_window_at(app, Some(anchor));
        } else {
            let _ = window.hide();
        }
    }
}

fn schedule_quick_window_hide<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let generation = state.quick_hide_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let callback_app = app.clone();
    let main_thread_app = callback_app.clone();
    thread::spawn(move || {
        thread::sleep(StdDuration::from_millis(120));
        let _ = callback_app.run_on_main_thread(move || {
            let Some(state) = main_thread_app.try_state::<AppState>() else {
                return;
            };
            if state.quick_hide_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Some(window) = main_thread_app.get_webview_window("quick")
                && !window.is_focused().unwrap_or(true)
            {
                let _ = window.hide();
            }
        });
    });
}

fn cancel_scheduled_quick_hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.quick_hide_generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn should_dismiss_quick_window(label: &str, focused: bool, quitting: bool) -> bool {
    label == "quick" && !focused && !quitting
}

fn next_window_visible(is_visible: bool) -> bool {
    !is_visible
}

fn position_quick_window<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>, anchor: Rect) {
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let anchor_position = anchor.position.to_physical::<i32>(scale_factor);
    let anchor_size = anchor.size.to_physical::<u32>(scale_factor);
    let monitor = app
        .monitor_from_point(f64::from(anchor_position.x), f64::from(anchor_position.y))
        .ok()
        .flatten();
    let current_panel_size = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(320, 500));
    let panel_size = monitor.as_ref().map_or(current_panel_size, |monitor| {
        physical_size_at_scale(current_panel_size, scale_factor, monitor.scale_factor())
    });
    let work_area = monitor.as_ref().map(|monitor| {
        let work_area = monitor.work_area();
        (work_area.position, work_area.size)
    });
    let origin = popover_origin(anchor_position, anchor_size, panel_size, work_area);
    let mut x = origin.x;
    let mut y = origin.y;

    if let Some(monitor) = monitor {
        let work_area = monitor.work_area();
        let margin = QUICK_WINDOW_MARGIN;
        let work_width = i32::try_from(work_area.size.width).unwrap_or(i32::MAX);
        let work_height = i32::try_from(work_area.size.height).unwrap_or(i32::MAX);
        let panel_width = i32::try_from(panel_size.width).unwrap_or(i32::MAX);
        let panel_height = i32::try_from(panel_size.height).unwrap_or(i32::MAX);
        let left = work_area.position.x + margin;
        let top = work_area.position.y + margin;
        let right = work_area.position.x + work_width - panel_width - margin;
        let bottom = work_area.position.y + work_height - panel_height - margin;
        x = x.clamp(left, right.max(left));
        y = y.clamp(top, bottom.max(top));
    }

    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn popover_origin(
    anchor_position: PhysicalPosition<i32>,
    anchor_size: PhysicalSize<u32>,
    panel_size: PhysicalSize<u32>,
    work_area: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
) -> PhysicalPosition<i32> {
    let anchor_width = i32::try_from(anchor_size.width).unwrap_or(i32::MAX);
    let panel_width = i32::try_from(panel_size.width).unwrap_or(i32::MAX);
    let panel_height = i32::try_from(panel_size.height).unwrap_or(i32::MAX);
    let x = anchor_position.x + (anchor_width - panel_width) / 2;
    let anchor_height = i32::try_from(anchor_size.height).unwrap_or(i32::MAX);
    let above = anchor_position
        .y
        .saturating_sub(panel_height)
        .saturating_sub(QUICK_WINDOW_MARGIN);
    let below = anchor_position
        .y
        .saturating_add(anchor_height)
        .saturating_add(QUICK_WINDOW_MARGIN);
    let y = if cfg!(target_os = "windows") {
        popover_y(above, below, panel_height, work_area)
    } else {
        below
    };
    PhysicalPosition::new(x, y)
}

fn popover_y(
    above: i32,
    below: i32,
    panel_height: i32,
    work_area: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
) -> i32 {
    let Some((work_position, work_size)) = work_area else {
        return above;
    };
    let work_top = work_position.y;
    let work_bottom = work_position
        .y
        .saturating_add(i32::try_from(work_size.height).unwrap_or(i32::MAX));
    let fits_above = above >= work_top;
    let fits_below = below <= work_bottom.saturating_sub(panel_height);

    if fits_above {
        above
    } else if fits_below {
        below
    } else {
        above
    }
}

fn update_tray_summary<R: Runtime>(app: &AppHandle<R>, summary: &QuickSummary) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tray_tooltip(summary)));
    }
}

fn tray_tooltip(summary: &QuickSummary) -> String {
    let today = summary
        .today_total_tokens
        .map_or_else(|| "Unavailable".to_owned(), |value| value.to_string());
    let today_cost = tray_cost(
        summary.today_provider_reported_cost,
        summary.today_estimated_cost,
    );
    let provider = summary.provider_name.as_deref().unwrap_or("Unavailable");
    format!(
        "TokenBuddy · 今日 {today} · 费用 {today_cost} · {} · Provider {provider}",
        summary.collection_status
    )
}

fn tray_cost(provider_reported_cost: Option<f64>, estimated_cost: Option<f64>) -> String {
    if let Some(value) = provider_reported_cost {
        return format!("${value:.4} USD");
    }
    if let Some(value) = estimated_cost {
        return format!("~${value:.4} USD");
    }
    "N/A".to_owned()
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        "open-quick" => show_quick_window(app),
        "open-dashboard" => show_window(app, "main"),
        "open-web" => {
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(mut server) = state.web_server.lock()
            {
                if server.is_none() {
                    if let Ok(web) =
                        start_local_web_server(app, Arc::clone(&state.core), resolve_web_root(app))
                    {
                        let url = web.status().url.clone();
                        *server = Some(web);
                        if let Some(url) = url {
                            let _ = open_url(&url);
                        }
                    }
                } else if let Some(url) = server.as_ref().and_then(|web| web.status().url) {
                    let _ = open_url(&url);
                }
            }
        }
        "rescan" => {
            if let Some(state) = app.try_state::<AppState>() {
                let _ = state.core.rescan_codex(None);
                let _ = state.core.rescan_claude(None);
            }
        }
        "quit" => {
            if let Some(state) = app.try_state::<AppState>() {
                state.quitting.store(true, Ordering::SeqCst);
                let _ = state.core.shutdown();
            }
            app.exit(0);
        }
        _ => {}
    }
}

fn handle_tray_event<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, event: TrayIconEvent) {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            rect,
            ..
        } => toggle_quick_window(tray.app_handle(), rect),
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => show_window(tray.app_handle(), "main"),
        _ => {}
    }
}

fn setup_tray<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = MenuBuilder::new(handle)
        .text("open-quick", "打开快速摘要")
        .text("open-dashboard", "打开完整面板")
        .text("open-web", "打开本地网页面板")
        .text("rescan", "立即导入 Codex + Claude")
        .separator()
        .text("quit", "退出 TokenBuddy")
        .build()?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("TokenBuddy · Core 正在采集")
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event);
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Build and run the desktop application.
///
/// Starts the Core, registers the tray, and keeps the windows hidden: the app
/// is a background collector that shows UI on demand, not a window that happens
/// to collect (spec §4.1). Blocks until the user quits from the tray.
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app, "main");
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?
                .join("tokenbuddy.sqlite3");
            let core = Core::start(
                CoreConfig::new(database_path, default_codex_home())
                    .with_claude_home(default_claude_home())
                    .with_cc_switch_db(default_cc_switch_db())
                    .with_cockpit_db(default_cockpit_db())
                    .with_official_quota_enabled(true),
            )
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            app.manage(AppState {
                core,
                web_server: Mutex::new(None),
                quitting: AtomicBool::new(false),
                quick_hide_generation: AtomicU64::new(0),
            });
            let settings = app
                .state::<AppState>()
                .core
                .get_app_settings()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            if settings.auto_start
                && let Err(error) = sync_autostart(app.handle(), true)
            {
                eprintln!("TokenBuddy autostart synchronization failed: {error}");
            }
            if debug_show_windows() {
                show_window(app.handle(), "main");
            } else {
                hide_window(app.handle(), "main");
                hide_window(app.handle(), "quick");
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            setup_tray(app)?;
            let app_handle = app.handle().clone();
            let core = Arc::clone(&app.state::<AppState>().core);
            core.add_summary_listener(move |summary| {
                let update_handle = app_handle.clone();
                let _ = app_handle.run_on_main_thread(move || {
                    update_tray_summary(&update_handle, &summary);
                });
            })
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            let initial_summary = core
                .quick_summary()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            update_tray_summary(app.handle(), &initial_summary);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let Some(state) = window.try_state::<AppState>() {
                match event {
                    WindowEvent::Focused(false)
                        if should_dismiss_quick_window(
                            window.label(),
                            false,
                            state.quitting.load(Ordering::SeqCst),
                        ) =>
                    {
                        schedule_quick_window_hide(window.app_handle());
                    }
                    WindowEvent::CloseRequested { api, .. }
                        if !state.quitting.load(Ordering::SeqCst)
                            && (window.label() == "main" || window.label() == "quick") =>
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_quick_summary,
            get_dashboard_summary,
            get_model_breakdown,
            export_usage,
            save_export,
            show_main_window,
            fit_quick_window_to_content,
            list_sessions,
            get_session_detail,
            list_usage_events,
            list_sources,
            list_providers,
            list_accounts,
            list_quota_snapshots,
            refresh_official_quota,
            pick_directory,
            pick_file,
            get_app_settings,
            update_app_settings,
            detect_codex_path,
            detect_official_quota_path,
            rescan_codex,
            detect_claude_path,
            rescan_claude,
            detect_cc_switch_path,
            rescan_cc_switch,
            detect_cockpit_path,
            rescan_cockpit,
            start_local_web_api,
            stop_local_web_api,
            get_local_web_api_status,
            open_local_web_api,
            quit_tokenbuddy
        ])
        .build(tauri::generate_context!());

    match result {
        Ok(app) => app.run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(state) = app_handle.try_state::<AppState>()
            {
                let _ = state.core.shutdown();
            }
        }),
        Err(error) => panic!("failed to build TokenBuddy: {error}"),
    }
}

fn debug_show_windows() -> bool {
    cfg!(debug_assertions)
        && std::env::var_os("TOKENBUDDY_DEBUG_SHOW_WINDOWS").is_some_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use tauri::{PhysicalPosition, PhysicalSize};

    use super::{
        debug_show_windows, fitted_quick_height, greet, max_quick_inner_height,
        next_window_visible, normalized_path, physical_size_at_scale, popover_origin, popover_y,
        should_dismiss_quick_window, tray_tooltip, windows_autostart_command_line,
    };

    #[test]
    fn greeting_mentions_the_name() {
        assert_eq!(greet("Codex"), "你好，Codex！TokenBuddy 的前后端通信正常。");
    }

    #[test]
    fn blank_custom_paths_fall_back_to_the_core_configuration() {
        assert!(normalized_path(Some("  ".to_owned())).is_none());
        assert_eq!(
            normalized_path(Some("/sanitized/codex".to_owned())).expect("path"),
            std::path::PathBuf::from("/sanitized/codex")
        );
    }

    #[test]
    fn windows_autostart_quotes_install_paths_with_spaces() {
        let executable =
            std::path::Path::new(r"C:\Program Files\TokenBuddy\tokenbuddy-desktop.exe");

        assert_eq!(
            windows_autostart_command_line(executable),
            r#""C:\Program Files\TokenBuddy\tokenbuddy-desktop.exe""#
        );
    }

    #[test]
    fn tray_tooltip_keeps_the_full_summary_off_the_status_bar() {
        let summary = tokenbuddy_domain::QuickSummary {
            collection_status: tokenbuddy_domain::CollectionStatus::Idle,
            active_app: None,
            active_session_id: None,
            active_session_title: None,
            active_project_path: None,
            provider_name: Some("OpenAI".to_owned()),
            model: None,
            session_input_tokens: None,
            session_cache_read_tokens: None,
            session_output_tokens: None,
            session_cache_hit_rate: None,
            session_provider_reported_cost: None,
            session_estimated_cost: None,
            today_total_tokens: Some(60_768_325),
            today_provider_reported_cost: None,
            today_estimated_cost: Some(3.8272),
            quota_summary: None,
            latest_warning: None,
        };

        assert_eq!(
            tray_tooltip(&summary),
            "TokenBuddy · 今日 60768325 · 费用 ~$3.8272 USD · idle · Provider OpenAI"
        );
    }

    #[test]
    fn tray_click_toggles_the_quick_panel_visibility() {
        assert!(next_window_visible(false));
        assert!(!next_window_visible(true));
    }

    #[test]
    fn quick_panel_dismisses_only_when_focus_leaves_the_quick_window() {
        assert!(should_dismiss_quick_window("quick", false, false));
        assert!(!should_dismiss_quick_window("quick", true, false));
        assert!(!should_dismiss_quick_window("main", false, false));
        assert!(!should_dismiss_quick_window("quick", false, true));
    }

    #[test]
    fn tray_popover_is_centered_on_the_anchor() {
        let origin = popover_origin(
            PhysicalPosition::new(1_000, 0),
            PhysicalSize::new(22, 24),
            PhysicalSize::new(360, 540),
            None,
        );
        assert_eq!(origin.x, 831);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(origin.y, 32);
        #[cfg(target_os = "windows")]
        assert_eq!(origin.y, -548);
    }

    #[test]
    fn windows_popover_moves_below_a_top_taskbar_when_above_does_not_fit() {
        let work_area = Some((PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080)));

        assert_eq!(popover_y(-528, 52, 540, work_area), 52);
    }

    #[test]
    fn windows_popover_stays_above_a_bottom_taskbar_when_both_sides_are_available() {
        let work_area = Some((PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080)));

        assert_eq!(popover_y(452, 1_032, 540, work_area), 452);
    }

    #[test]
    fn tray_popover_uses_the_target_monitor_scale_for_placement() {
        assert_eq!(
            physical_size_at_scale(PhysicalSize::new(320, 420), 1.0, 1.5),
            PhysicalSize::new(480, 630)
        );
        assert_eq!(
            physical_size_at_scale(PhysicalSize::new(480, 630), 1.5, 1.0),
            PhysicalSize::new(320, 420)
        );
    }

    #[test]
    fn quick_panel_height_is_capped_to_the_target_work_area() {
        let maximum = max_quick_inner_height(600, 420, 432, 1.0, 1.5);

        assert_eq!(maximum, Some(377.0));
        assert_eq!(fitted_quick_height(800.0, maximum), 377.0);
        assert_eq!(fitted_quick_height(300.0, maximum), 300.0);
        assert_eq!(fitted_quick_height(40.0, maximum), 120.0);
        assert_eq!(fitted_quick_height(800.0, None), 800.0);
    }

    #[test]
    fn debug_window_switch_is_opt_in() {
        assert!(!debug_show_windows());
    }
}

/// Exercises the `#[tauri::command]` layer itself.
///
/// The command functions are the contract the desktop panel actually calls, but
/// they were previously untested: the unit tests above only reached the pure
/// helpers around them. Tauri's mock runtime provides a real `State` without a
/// window server, so every command that only needs state can be driven here —
/// which also pins the argument defaults (page sizes, empty filters) that the
/// frontend relies on.
///
/// Commands taking an `AppHandle` (`save_export`, `show_main_window`, the
/// pickers, `quit_tokenbuddy`) are deliberately absent: they are bound to the
/// real runtime, and `quit_tokenbuddy` would end the test process.
// Tauri's MockRuntime test binary currently exits with STATUS_ENTRYPOINT_NOT_FOUND
// on windows-latest before the first test starts. Keep the command-contract
// suite on the platforms where the mock runtime is loadable; Windows still
// runs the pure desktop tests and the full Tauri Windows build in CI.
#[cfg(all(test, not(windows)))]
mod command_tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicU64},
        },
    };

    use tauri::{Manager, State};
    use tempfile::TempDir;
    use tokenbuddy_core::{Core, CoreConfig};
    use tokenbuddy_domain::UsageFilters;

    use super::{
        AppState, detect_cc_switch_path, detect_claude_path, detect_cockpit_path,
        detect_codex_path, export_usage, get_app_settings, get_dashboard_summary,
        get_local_web_api_status, get_model_breakdown, get_quick_summary, get_session_detail,
        list_accounts, list_providers, list_quota_snapshots, list_sessions, list_sources,
        list_usage_events, rescan_cc_switch, rescan_claude, rescan_cockpit, rescan_codex,
        start_directory, stop_local_web_api,
    };

    struct Harness {
        app: tauri::App<tauri::test::MockRuntime>,
        _codex_home: TempDir,
        _database: TempDir,
    }

    impl Harness {
        /// A mock app managing a real Core over a sanitized Codex fixture.
        fn new() -> Self {
            let codex_home = tempfile::tempdir().expect("codex home");
            let sessions = codex_home.path().join("sessions");
            fs::create_dir_all(&sessions).expect("sessions directory");
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../fixtures/codex/simple_session.jsonl");
            fs::copy(fixture, sessions.join("simple_session.jsonl")).expect("copy fixture");

            let database = tempfile::tempdir().expect("database directory");
            let core = Core::start(CoreConfig::new(
                database.path().join("tokenbuddy.sqlite3"),
                Some(codex_home.path().to_owned()),
            ))
            .expect("core starts");

            let app = tauri::test::mock_app();
            app.manage(AppState {
                core,
                web_server: Mutex::new(None),
                quitting: AtomicBool::new(false),
                quick_hide_generation: AtomicU64::new(0),
            });
            Self {
                app,
                _codex_home: codex_home,
                _database: database,
            }
        }

        fn state(&self) -> State<'_, AppState> {
            self.app.state()
        }

        fn codex_home(&self) -> PathBuf {
            self._codex_home.path().to_owned()
        }
    }

    #[test]
    fn read_commands_return_the_core_owned_view_of_the_fixture() {
        let harness = Harness::new();

        let summary = get_quick_summary(harness.state()).expect("quick summary");
        assert_eq!(summary.active_app, Some(tokenbuddy_domain::AppKind::Codex));
        assert_eq!(summary.model.as_deref(), Some("gpt-5-codex"));

        // The dashboard defaults to today's local window when no filter is given.
        let dashboard = get_dashboard_summary(harness.state(), None).expect("dashboard");
        assert!(dashboard.period_start < dashboard.period_end);

        let breakdown = get_model_breakdown(harness.state(), None).expect("model breakdown");
        assert!(
            breakdown
                .iter()
                .all(|usage| usage.app == tokenbuddy_domain::AppKind::Codex)
        );

        // The fixture holds one session with two usage events.
        let sessions = list_sessions(harness.state(), None, None, None).expect("sessions");
        assert_eq!(sessions.total, 1);
        let session_id = sessions.sessions[0].session.id.clone();

        let detail = get_session_detail(harness.state(), session_id.clone())
            .expect("session detail")
            .expect("session exists");
        assert_eq!(detail.summary.session.id, session_id);
        assert_eq!(detail.usage_events.len(), 2);

        let events = list_usage_events(harness.state(), None, None, None).expect("usage events");
        assert_eq!(events.total, 2);
        let scoped = list_usage_events(harness.state(), Some(session_id), None, None)
            .expect("scoped usage events");
        assert_eq!(scoped.total, 2);

        let sources = list_sources(harness.state()).expect("sources");
        assert!(sources.iter().any(|source| source.id == "codex-session"));
        assert!(
            !list_providers(harness.state())
                .expect("providers")
                .is_empty()
        );

        // No account or quota source is configured for this fixture, and the
        // commands say so with empty results rather than inventing rows.
        assert!(
            list_accounts(harness.state())
                .expect("accounts")
                .iter()
                .all(|summary| summary.account.auth_mode == "session_log")
        );
        assert!(
            list_quota_snapshots(harness.state(), None, None)
                .expect("quotas")
                .is_empty()
        );
    }

    #[test]
    fn missing_session_detail_is_absent_rather_than_an_error() {
        let harness = Harness::new();
        assert!(
            get_session_detail(harness.state(), "codex-session:does-not-exist".to_owned())
                .expect("command succeeds")
                .is_none()
        );
    }

    #[test]
    fn export_commands_cover_both_formats_and_reject_unknown_ones() {
        let harness = Harness::new();

        let csv = export_usage(harness.state(), "csv".to_owned(), None).expect("csv export");
        assert_eq!(csv.mime_type, "text/csv;charset=utf-8");
        assert!(csv.filename.ends_with(".csv"));
        assert!(csv.content.contains("occurred_at"));
        // Exports carry no raw payload.
        assert!(!csv.content.contains("raw_usage_json"));

        let json = export_usage(
            harness.state(),
            "json".to_owned(),
            Some(UsageFilters::default()),
        )
        .expect("json export");
        assert_eq!(json.mime_type, "application/json");

        let rejected = export_usage(harness.state(), "pdf".to_owned(), None);
        assert!(
            rejected.is_err(),
            "unknown formats must not silently fall back"
        );
    }

    #[test]
    fn detection_commands_report_configured_and_missing_sources_explicitly() {
        let harness = Harness::new();
        let home = harness.codex_home();

        let codex = detect_codex_path(harness.state(), None).expect("codex detection");
        assert!(codex.detected);
        let codex_custom =
            detect_codex_path(harness.state(), Some(home.to_string_lossy().into_owned()))
                .expect("codex detection with an explicit path");
        assert!(codex_custom.detected);

        // Nothing else is configured on this fixture machine, so each source
        // reports "not found" instead of erroring or claiming success.
        for detection in [
            detect_claude_path(harness.state(), None).expect("claude detection"),
            detect_cc_switch_path(harness.state(), None).expect("cc-switch detection"),
            detect_cockpit_path(harness.state(), None).expect("cockpit detection"),
        ] {
            assert!(!detection.detected);
        }

        let absent = harness
            .codex_home()
            .join("absent")
            .to_string_lossy()
            .into_owned();
        assert!(
            !detect_cc_switch_path(harness.state(), Some(absent.clone()))
                .expect("cc-switch detection with an explicit path")
                .detected
        );
        assert!(
            !detect_cockpit_path(harness.state(), Some(absent))
                .expect("cockpit detection with an explicit path")
                .detected
        );
    }

    #[test]
    fn rescan_commands_are_idempotent_over_the_same_fixture() {
        let harness = Harness::new();

        // The Core already imported at startup, so a rescan adds nothing.
        let codex = rescan_codex(harness.state(), None).expect("codex rescan");
        assert_eq!(codex.inserted_events, 0);

        // Sources that are not configured still return a report rather than an
        // error — one missing launcher must not break the scan button.
        for report in [
            rescan_claude(harness.state(), None).expect("claude rescan"),
            rescan_cc_switch(harness.state(), None).expect("cc-switch rescan"),
            rescan_cockpit(harness.state(), None).expect("cockpit rescan"),
        ] {
            assert_eq!(report.inserted_events, 0);
        }

        assert_eq!(
            list_usage_events(harness.state(), None, None, None)
                .expect("usage events")
                .total,
            2,
            "repeated scans must not change the event count"
        );
    }

    /// `update_app_settings` is intentionally not driven here: it is bound to
    /// the real runtime and calls `sync_autostart`, which would register a login
    /// item on the machine running the tests. The Core-level round trip is
    /// covered in `tokenbuddy-core`; this pins what the read command exposes.
    #[test]
    fn the_settings_command_exposes_the_core_configuration() {
        let harness = Harness::new();

        let settings = get_app_settings(harness.state()).expect("settings");
        assert_eq!(
            settings.codex_home.as_deref(),
            Some(harness.codex_home().to_string_lossy().as_ref())
        );
        // Everything the app has not been told about stays unset rather than
        // defaulting to something that looks configured.
        assert_eq!(settings.claude_home, None);
        assert_eq!(settings.cc_switch_db_path, None);
        assert_eq!(settings.cockpit_path, None);
        assert_eq!(settings.otel_port, None);
        assert_eq!(settings.data_retention_days, None);
        assert!(!settings.proxy_enabled);
        assert!(!settings.auto_start);
    }

    #[test]
    fn the_local_web_api_reports_stopped_until_it_is_started() {
        let harness = Harness::new();

        let status = get_local_web_api_status(harness.state()).expect("status");
        assert!(!status.running);
        assert!(status.url.is_none());
        assert!(status.loopback_urls.is_empty());

        // Stopping an already-stopped server is not an error.
        let stopped = stop_local_web_api(harness.state()).expect("stop");
        assert!(!stopped.running);
    }

    #[test]
    fn the_picker_opens_at_the_configured_location_and_ignores_stale_paths() {
        let directory = tempfile::tempdir().expect("directory");
        let file = directory.path().join("cc-switch.db");
        fs::write(&file, b"").expect("file");

        // A directory opens itself; a file opens its parent.
        assert_eq!(
            start_directory(Some(directory.path().to_string_lossy().into_owned())),
            Some(directory.path().to_owned())
        );
        assert_eq!(
            start_directory(Some(file.to_string_lossy().into_owned())),
            Some(directory.path().to_owned())
        );
        // A path that no longer exists falls back to the system default rather
        // than failing the picker.
        assert_eq!(
            start_directory(Some(
                directory
                    .path()
                    .join("gone/deeper")
                    .to_string_lossy()
                    .into_owned()
            )),
            None
        );
        assert_eq!(start_directory(None), None);
        assert_eq!(start_directory(Some("   ".to_owned())), None);
    }
}
