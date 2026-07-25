use std::{path::PathBuf, sync::Mutex};

use chrono::{Datelike, Duration, TimeZone, Utc};
use tauri::{Manager, State};
use tokenbuddy_codex_session::{CodexSessionAdapter, default_codex_home};
use tokenbuddy_domain::{
    DashboardSummary, DetectionResult, SessionDetail, SessionPage, UsageAdapter,
};
use tokenbuddy_storage::{Database, StorageError};

struct AppState {
    database: Mutex<Database>,
}

#[derive(Debug, serde::Serialize)]
struct RescanResult {
    inserted_events: u64,
    duplicate_events: u64,
    upserted_sessions: u64,
    updated_cursors: u64,
    skipped_records: usize,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("你好，{name}！TokenBuddy 的前后端通信正常。")
}

#[tauri::command]
fn get_dashboard_summary(state: State<'_, AppState>) -> Result<DashboardSummary, String> {
    let now = Utc::now();
    let period_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .ok_or_else(|| "无法计算今日统计窗口".to_owned())?;
    let period_end = period_start + Duration::days(1);
    let database = lock_database(&state)?;
    database
        .dashboard_summary(period_start, period_end)
        .map_err(storage_error)
}

#[tauri::command]
fn list_sessions(
    state: State<'_, AppState>,
    search: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<SessionPage, String> {
    let database = lock_database(&state)?;
    database
        .list_session_page(search.as_deref(), limit.unwrap_or(50), offset.unwrap_or(0))
        .map_err(storage_error)
}

#[tauri::command]
fn get_session_detail(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<SessionDetail>, String> {
    let database = lock_database(&state)?;
    database
        .get_session_detail(&session_id)
        .map_err(storage_error)
}

#[tauri::command]
fn list_usage_events(
    state: State<'_, AppState>,
    session_id: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<tokenbuddy_domain::UsageEventPage, String> {
    let database = lock_database(&state)?;
    database
        .list_usage_events(
            session_id.as_deref(),
            limit.unwrap_or(100),
            offset.unwrap_or(0),
        )
        .map_err(storage_error)
}

#[tauri::command]
fn list_sources(
    state: State<'_, AppState>,
) -> Result<Vec<tokenbuddy_domain::SourceRecord>, String> {
    let database = lock_database(&state)?;
    database.list_sources().map_err(storage_error)
}

#[tauri::command]
fn detect_codex_path(codex_home: Option<String>) -> Result<DetectionResult, String> {
    let home = resolve_codex_home(codex_home)?;
    CodexSessionAdapter::new(home)
        .detect_sync()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn rescan_codex(
    state: State<'_, AppState>,
    codex_home: Option<String>,
) -> Result<RescanResult, String> {
    let home = resolve_codex_home(codex_home)?;
    let adapter = CodexSessionAdapter::new(home);
    let mut database = lock_database(&state)?;
    let cursors = database
        .list_import_cursors(adapter.id())
        .map_err(storage_error)?;
    let batch = adapter
        .import_history_sync(&cursors)
        .map_err(|error| error.to_string())?;
    let skipped_records = batch.skipped_records;
    let stats = database.apply_import_batch(&batch).map_err(storage_error)?;
    Ok(RescanResult {
        inserted_events: stats.inserted_events,
        duplicate_events: stats.duplicate_events,
        upserted_sessions: stats.upserted_sessions,
        updated_cursors: stats.updated_cursors,
        skipped_records,
    })
}

fn resolve_codex_home(codex_home: Option<String>) -> Result<PathBuf, String> {
    codex_home
        .map(|value| PathBuf::from(value.trim()))
        .filter(|value| !value.as_os_str().is_empty())
        .or_else(default_codex_home)
        .ok_or_else(|| "未找到 Codex Home，请在设置中提供路径".to_owned())
}

fn lock_database<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, Database>, String> {
    state
        .database
        .lock()
        .map_err(|_| "数据库锁已损坏，请重启 TokenBuddy".to_owned())
}

fn storage_error(error: StorageError) -> String {
    error.to_string()
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?
                .join("tokenbuddy.sqlite3");
            let database = Database::open(database_path)
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            app.manage(AppState {
                database: Mutex::new(database),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_dashboard_summary,
            list_sessions,
            get_session_detail,
            list_usage_events,
            list_sources,
            detect_codex_path,
            rescan_codex
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TokenBuddy");
}

#[cfg(test)]
mod tests {
    use super::greet;

    #[test]
    fn greeting_mentions_the_name() {
        assert_eq!(greet("Codex"), "你好，Codex！TokenBuddy 的前后端通信正常。");
    }
}
