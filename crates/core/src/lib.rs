//! The single long-lived application core shared by every TokenBuddy entry.
//!
//! The core owns the SQLite connection, the Codex incremental importer, and a
//! small pre-aggregated summary. UI surfaces only call query methods on this
//! type; they never scan source files or open SQLite themselves.

use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokenbuddy_codex_session::{CodexSessionAdapter, SOURCE_ID};
use tokenbuddy_domain::{
    CollectionStatus, DashboardSummary, DetectionResult, QuickSummary, SessionDetail, SessionPage,
    SourceRecord, UsageAdapter, UsageEventPage,
};
use tokenbuddy_storage::{Database, ImportStats, StorageError};

type SummaryListener = Arc<dyn Fn(QuickSummary) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub database_path: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub poll_interval: Duration,
}

impl CoreConfig {
    pub fn new(database_path: impl Into<PathBuf>, codex_home: Option<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            codex_home,
            poll_interval: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("Codex adapter error: {0}")]
    Adapter(String),
    #[error("core lock is poisoned: {0}")]
    Lock(&'static str),
    #[error("failed to start core worker: {0}")]
    Worker(#[from] std::io::Error),
    #[error("core worker did not stop cleanly")]
    WorkerDidNotStop,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub inserted_events: u64,
    pub duplicate_events: u64,
    pub upserted_sessions: u64,
    pub updated_cursors: u64,
    pub skipped_records: usize,
    pub warning: Option<String>,
}

impl From<ImportStats> for ImportReport {
    fn from(stats: ImportStats) -> Self {
        Self {
            inserted_events: stats.inserted_events,
            duplicate_events: stats.duplicate_events,
            upserted_sessions: stats.upserted_sessions,
            updated_cursors: stats.updated_cursors,
            ..Self::default()
        }
    }
}

struct CoreControl {
    stop: AtomicBool,
    wake: Condvar,
    wake_lock: Mutex<bool>,
}

impl CoreControl {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            wake: Condvar::new(),
            wake_lock: Mutex::new(false),
        }
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }

    fn wake(&self) {
        self.wake.notify_all();
    }

    fn wait(&self, timeout: Duration) {
        let Ok(wake_lock) = self.wake_lock.lock() else {
            return;
        };
        let _ = self.wake.wait_timeout(wake_lock, timeout);
    }
}

pub struct Core {
    database: Mutex<Database>,
    import_lock: Mutex<()>,
    codex_home: RwLock<Option<PathBuf>>,
    summary: RwLock<QuickSummary>,
    summary_listeners: Mutex<Vec<SummaryListener>>,
    control: Arc<CoreControl>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Core {
    pub fn start(config: CoreConfig) -> Result<Arc<Self>, CoreError> {
        let database = Database::open(&config.database_path)?;
        let control = Arc::new(CoreControl::new());
        let core = Arc::new(Self {
            database: Mutex::new(database),
            import_lock: Mutex::new(()),
            codex_home: RwLock::new(config.codex_home),
            summary: RwLock::new(QuickSummary::starting()),
            summary_listeners: Mutex::new(Vec::new()),
            control: Arc::clone(&control),
            worker: Mutex::new(None),
        });

        // Do one synchronous pass so the tray summary is useful immediately;
        // subsequent passes are owned by the one worker below.
        core.refresh_once()?;

        let weak_core = Arc::downgrade(&core);
        let poll_interval = config.poll_interval;
        let worker = thread::Builder::new()
            .name("tokenbuddy-core".to_owned())
            .spawn(move || worker_loop(weak_core, control, poll_interval))?;
        core.worker
            .lock()
            .map_err(|_| CoreError::Lock("worker"))?
            .replace(worker);
        Ok(core)
    }

    pub fn quick_summary(&self) -> Result<QuickSummary, CoreError> {
        self.summary
            .read()
            .map(|summary| summary.clone())
            .map_err(|_| CoreError::Lock("summary"))
    }

    pub fn add_summary_listener<F>(&self, listener: F) -> Result<(), CoreError>
    where
        F: Fn(QuickSummary) + Send + Sync + 'static,
    {
        self.summary_listeners
            .lock()
            .map_err(|_| CoreError::Lock("summary listeners"))?
            .push(Arc::new(listener));
        Ok(())
    }

    pub fn rescan_codex(&self, codex_home: Option<PathBuf>) -> Result<ImportReport, CoreError> {
        if let Some(codex_home) = codex_home {
            self.set_codex_home(Some(codex_home))?;
        }
        self.refresh_once()
    }

    pub fn set_codex_home(&self, codex_home: Option<PathBuf>) -> Result<(), CoreError> {
        *self
            .codex_home
            .write()
            .map_err(|_| CoreError::Lock("Codex home"))? = codex_home;
        self.control.wake();
        Ok(())
    }

    pub fn codex_home(&self) -> Result<Option<PathBuf>, CoreError> {
        self.codex_home
            .read()
            .map(|path| path.clone())
            .map_err(|_| CoreError::Lock("Codex home"))
    }

    pub fn detect_codex_path(&self) -> Result<DetectionResult, CoreError> {
        let Some(home) = self.codex_home()? else {
            return Ok(DetectionResult {
                source_id: SOURCE_ID.to_owned(),
                detected: false,
                path_or_endpoint: None,
                detected_version: None,
                message: Some("未配置 Codex Home".to_owned()),
            });
        };
        CodexSessionAdapter::new(home)
            .detect_sync()
            .map_err(|error| CoreError::Adapter(error.to_string()))
    }

    pub fn dashboard_summary(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<DashboardSummary, CoreError> {
        self.database_lock()?
            .dashboard_summary(period_start, period_end)
            .map_err(CoreError::from)
    }

    pub fn today_dashboard_summary(&self) -> Result<DashboardSummary, CoreError> {
        let now = Utc::now();
        let period_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|value| DateTime::<Utc>::from_naive_utc_and_offset(value, Utc))
            .ok_or_else(|| CoreError::Adapter("无法计算今日统计窗口".to_owned()))?;
        self.dashboard_summary(period_start, period_start + chrono::Duration::days(1))
    }

    pub fn list_sessions(
        &self,
        search: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<SessionPage, CoreError> {
        self.database_lock()?
            .list_session_page(search, limit, offset)
            .map_err(CoreError::from)
    }

    pub fn get_session_detail(&self, session_id: &str) -> Result<Option<SessionDetail>, CoreError> {
        self.database_lock()?
            .get_session_detail(session_id)
            .map_err(CoreError::from)
    }

    pub fn list_usage_events(
        &self,
        session_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<UsageEventPage, CoreError> {
        self.database_lock()?
            .list_usage_events(session_id, limit, offset)
            .map_err(CoreError::from)
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRecord>, CoreError> {
        self.database_lock()?
            .list_sources()
            .map_err(CoreError::from)
    }

    pub fn shutdown(&self) -> Result<(), CoreError> {
        self.control.request_stop();
        let worker = self
            .worker
            .lock()
            .map_err(|_| CoreError::Lock("worker"))?
            .take();
        if let Some(worker) = worker {
            worker.join().map_err(|_| CoreError::WorkerDidNotStop)?;
        }
        Ok(())
    }

    fn database_lock(&self) -> Result<std::sync::MutexGuard<'_, Database>, CoreError> {
        self.database
            .lock()
            .map_err(|_| CoreError::Lock("database"))
    }

    fn refresh_once(&self) -> Result<ImportReport, CoreError> {
        let _import_guard = self
            .import_lock
            .lock()
            .map_err(|_| CoreError::Lock("import"))?;
        let Some(codex_home) = self.codex_home()? else {
            self.refresh_summary(
                CollectionStatus::Idle,
                Some("未配置 Codex Home；Codex 数据源保持 Unavailable".to_owned()),
            )?;
            return Ok(ImportReport::default());
        };

        let adapter = CodexSessionAdapter::new(codex_home);
        let cursors = self.database_lock()?.list_import_cursors(adapter.id())?;
        let batch = match adapter.import_history_sync(&cursors) {
            Ok(batch) => batch,
            Err(error) => {
                let message = error.to_string();
                self.refresh_summary(CollectionStatus::Error, Some(message.clone()))?;
                return Ok(ImportReport {
                    warning: Some(message),
                    ..ImportReport::default()
                });
            }
        };
        let warning = (batch.skipped_records > 0)
            .then(|| format!("Codex 导入跳过 {} 条无法解析的记录", batch.skipped_records));
        let stats = self.database_lock()?.apply_import_batch(&batch)?;
        self.refresh_summary(
            if batch
                .source
                .as_ref()
                .and_then(|source| source.health_status.as_deref())
                == Some("not_found")
            {
                CollectionStatus::Idle
            } else {
                CollectionStatus::Collecting
            },
            warning.clone(),
        )?;
        Ok(ImportReport {
            skipped_records: batch.skipped_records,
            warning,
            ..ImportReport::from(stats)
        })
    }

    fn refresh_summary(
        &self,
        status: CollectionStatus,
        warning: Option<String>,
    ) -> Result<(), CoreError> {
        let summary = self
            .database_lock()?
            .quick_summary(Utc::now(), status, warning)?;
        *self
            .summary
            .write()
            .map_err(|_| CoreError::Lock("summary"))? = summary.clone();
        self.notify_summary(summary)
    }

    fn notify_summary(&self, summary: QuickSummary) -> Result<(), CoreError> {
        let listeners = self
            .summary_listeners
            .lock()
            .map_err(|_| CoreError::Lock("summary listeners"))?
            .clone();
        for listener in listeners {
            listener(summary.clone());
        }
        Ok(())
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.control.request_stop();
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    weak_core: std::sync::Weak<Core>,
    control: Arc<CoreControl>,
    poll_interval: Duration,
) {
    loop {
        if control.stop.load(Ordering::SeqCst) {
            break;
        }
        if let Some(core) = weak_core.upgrade() {
            let _ = core.refresh_once();
        } else {
            break;
        }
        control.wait(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write, path::Path, thread, time::Instant};

    use chrono::Utc;
    use tempfile::TempDir;

    use super::{Core, CoreConfig};

    fn fixture_home(fixture: &str) -> (TempDir, std::path::PathBuf) {
        let home = tempfile::tempdir().expect("temporary home");
        let sessions = home.path().join("sessions");
        fs::create_dir_all(&sessions).expect("sessions directory");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/codex")
            .join(fixture);
        let destination = sessions.join(fixture);
        fs::copy(fixture_path, &destination).expect("copy fixture");
        (home, destination)
    }

    #[test]
    fn core_imports_in_the_background_and_updates_quick_summary() {
        let (home, session_path) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let mut config = CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        );
        config.poll_interval = std::time::Duration::from_millis(10);
        let core = Core::start(config).expect("core starts");
        let initial = core.list_usage_events(None, 50, 0).expect("initial events");
        assert_eq!(initial.total, 2);

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(session_path)
            .expect("open fixture");
        writeln!(
            file,
            "{{\"type\":\"response.completed\",\"session_id\":\"simple-session\",\"timestamp\":\"{}\",\"request_id\":\"background-request\",\"model\":\"gpt-5-codex\",\"usage\":{{\"input_tokens\":20,\"output_tokens\":8}}}}",
            Utc::now().to_rfc3339()
        )
        .expect("append fixture record");

        let started = Instant::now();
        loop {
            if core
                .list_usage_events(None, 50, 0)
                .expect("poll events")
                .total
                == 3
            {
                break;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
            thread::sleep(std::time::Duration::from_millis(15));
        }

        let summary = core.quick_summary().expect("quick summary");
        assert_eq!(summary.active_app, Some(tokenbuddy_domain::AppKind::Codex));
        assert_eq!(summary.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(summary.session_output_tokens, Some(78));
        assert_eq!(summary.today_total_tokens, Some(28));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn repeated_core_shutdown_is_safe() {
        let (home, _) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        ))
        .expect("core starts");
        core.shutdown().expect("first shutdown");
        core.shutdown().expect("second shutdown");
    }
}
