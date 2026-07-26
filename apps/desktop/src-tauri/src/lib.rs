mod web;

use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{Datelike, Duration, TimeZone, Utc};
use tauri::{
    App, AppHandle, Manager, Runtime, State, WindowEvent,
    menu::{MenuBuilder, MenuEvent},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokenbuddy_codex_session::{CodexSessionAdapter, default_codex_home};
use tokenbuddy_core::{Core, CoreConfig, CoreError, ImportReport};
use tokenbuddy_domain::{
    DashboardSummary, DetectionResult, QuickSummary, SessionDetail, SessionPage,
};
use web::{LocalWebApiStatus, LocalWebServer};

struct AppState {
    core: Arc<Core>,
    web_server: Mutex<Option<LocalWebServer>>,
    quitting: AtomicBool,
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
fn get_dashboard_summary(state: State<'_, AppState>) -> Result<DashboardSummary, String> {
    let (period_start, period_end) = today_period()?;
    state
        .core
        .dashboard_summary(period_start, period_end)
        .map_err(core_error)
}

#[tauri::command]
fn list_sessions(
    state: State<'_, AppState>,
    search: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<SessionPage, String> {
    state
        .core
        .list_sessions(search.as_deref(), limit.unwrap_or(50), offset.unwrap_or(0))
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
    let server = LocalWebServer::start(Arc::clone(&state.core), static_root)
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

fn today_period() -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>), String> {
    let now = Utc::now();
    let start = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .ok_or_else(|| "无法计算今日统计窗口".to_owned())?;
    Ok((start, start + Duration::days(1)))
}

fn show_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.hide();
    }
}

fn update_tray_summary<R: Runtime>(app: &AppHandle<R>, summary: &QuickSummary) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tray_tooltip(summary)));
        #[cfg(target_os = "macos")]
        {
            let today = summary
                .today_total_tokens
                .map_or_else(|| "Unavailable".to_owned(), |value| value.to_string());
            let _ = tray.set_title(Some(format!("Today {today}")));
        }
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
        "open-quick" => show_window(app, "quick"),
        "open-dashboard" => show_window(app, "main"),
        "open-web" => {
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(mut server) = state.web_server.lock()
            {
                if server.is_none() {
                    if let Ok(web) =
                        LocalWebServer::start(Arc::clone(&state.core), resolve_web_root(app))
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
            ..
        } => show_window(tray.app_handle(), "quick"),
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
        .text("rescan", "立即导入 Codex")
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
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?
                .join("tokenbuddy.sqlite3");
            let core = Core::start(CoreConfig::new(database_path, default_codex_home()))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            app.manage(AppState {
                core,
                web_server: Mutex::new(None),
                quitting: AtomicBool::new(false),
            });
            hide_window(app.handle(), "main");
            hide_window(app.handle(), "quick");
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
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
            if let WindowEvent::CloseRequested { api, .. } = event
                && let Some(state) = window.try_state::<AppState>()
                && !state.quitting.load(Ordering::SeqCst)
                && (window.label() == "main" || window.label() == "quick")
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_quick_summary,
            get_dashboard_summary,
            list_sessions,
            get_session_detail,
            list_usage_events,
            list_sources,
            detect_codex_path,
            rescan_codex,
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

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::{greet, normalized_path, today_period};

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
    fn today_period_is_a_full_utc_day() {
        let (start, end) = today_period().expect("period");
        assert_eq!(end - start, chrono::Duration::days(1));
        assert_eq!(start.hour(), 0);
    }
}
