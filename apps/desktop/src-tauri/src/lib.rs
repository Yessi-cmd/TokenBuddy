mod web;

use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration as StdDuration,
};

use tauri::{
    App, AppHandle, Manager, PhysicalPosition, PhysicalSize, Rect, Runtime, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
    menu::{MenuBuilder, MenuEvent},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Effect, EffectState, EffectsBuilder},
};
use tokenbuddy_claude_session::{ClaudeSessionAdapter, default_claude_home};
use tokenbuddy_codex_session::{CodexSessionAdapter, default_codex_home};
use tokenbuddy_core::{Core, CoreConfig, CoreError, ImportReport};
use tokenbuddy_domain::{
    AppSettings, DashboardSummary, DetectionResult, ExportResult, QuickSummary, SessionDetail,
    SessionPage, UsageFilters,
};
use web::{AutostartCallback, LocalWebApiStatus, LocalWebServer};

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

fn show_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(window) = get_or_create_window(app, label) {
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
        .inner_size(320.0, 500.0)
        .min_inner_size(300.0, 460.0)
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
    let panel_size = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(320, 500));
    let origin = popover_origin(anchor_position, anchor_size, panel_size);
    let mut x = origin.x;
    let mut y = origin.y;

    if let Ok(Some(monitor)) =
        app.monitor_from_point(f64::from(anchor_position.x), f64::from(anchor_position.y))
    {
        let work_area = monitor.work_area();
        let margin = 8;
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
) -> PhysicalPosition<i32> {
    let anchor_width = i32::try_from(anchor_size.width).unwrap_or(i32::MAX);
    let panel_width = i32::try_from(panel_size.width).unwrap_or(i32::MAX);
    #[cfg(target_os = "windows")]
    let panel_height = i32::try_from(panel_size.height).unwrap_or(i32::MAX);
    let x = anchor_position.x + (anchor_width - panel_width) / 2;
    #[cfg(target_os = "windows")]
    let y = anchor_position.y - panel_height - 8;
    #[cfg(not(target_os = "windows"))]
    let y = anchor_position.y + i32::try_from(anchor_size.height).unwrap_or(i32::MAX) + 8;
    PhysicalPosition::new(x, y)
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
    let provider = summary.provider_name.as_deref().unwrap_or("Unavailable");
    format!(
        "TokenBuddy · 今日 {today} · {} · Provider {provider}",
        summary.collection_status
    )
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

pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app, "main");
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?
                .join("tokenbuddy.sqlite3");
            let core = Core::start(
                CoreConfig::new(database_path, default_codex_home())
                    .with_claude_home(default_claude_home()),
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
            export_usage,
            save_export,
            show_main_window,
            list_sessions,
            get_session_detail,
            list_usage_events,
            list_sources,
            list_providers,
            list_quota_snapshots,
            get_app_settings,
            update_app_settings,
            detect_codex_path,
            rescan_codex,
            detect_claude_path,
            rescan_claude,
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
        debug_show_windows, greet, next_window_visible, normalized_path, popover_origin,
        should_dismiss_quick_window, tray_tooltip,
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
            today_total_tokens: Some(60_768_325),
            quota_summary: None,
            latest_warning: None,
        };

        assert_eq!(
            tray_tooltip(&summary),
            "TokenBuddy · 今日 60768325 · idle · Provider OpenAI"
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
        );
        assert_eq!(origin.x, 831);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(origin.y, 32);
        #[cfg(target_os = "windows")]
        assert_eq!(origin.y, -548);
    }

    #[test]
    fn debug_window_switch_is_opt_in() {
        assert!(!debug_show_windows());
    }
}
