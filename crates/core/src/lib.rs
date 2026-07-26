//! The single long-lived application core shared by every TokenBuddy entry.
//!
//! The core owns the SQLite connection, the Codex and Claude incremental
//! importers, and a small pre-aggregated summary. UI surfaces only call query
//! methods on this type; they never scan source files or open SQLite.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, TimeZone, Utc};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;
use tokenbuddy_cc_switch::{
    ADAPTER_TYPE as CC_SWITCH_ADAPTER_TYPE, CcSwitchAdapter,
    DISPLAY_NAME as CC_SWITCH_DISPLAY_NAME, SOURCE_ID as CC_SWITCH_SOURCE_ID,
};
use tokenbuddy_claude_session::{
    ADAPTER_TYPE as CLAUDE_ADAPTER_TYPE, ClaudeSessionAdapter, DISPLAY_NAME as CLAUDE_DISPLAY_NAME,
    SOURCE_ID as CLAUDE_SOURCE_ID,
};
use tokenbuddy_cockpit::{
    ADAPTER_TYPE as COCKPIT_ADAPTER_TYPE, CockpitAdapter, DISPLAY_NAME as COCKPIT_DISPLAY_NAME,
    SOURCE_ID as COCKPIT_SOURCE_ID,
};
use tokenbuddy_codex_session::{
    ADAPTER_TYPE as CODEX_ADAPTER_TYPE, CodexSessionAdapter, DISPLAY_NAME as CODEX_DISPLAY_NAME,
    SOURCE_ID as CODEX_SOURCE_ID,
};
use tokenbuddy_domain::{
    AppSettings, CollectionStatus, DashboardSummary, DetectionResult, ExportResult, ImportBatch,
    QuickSummary, SessionDetail, SessionPage, SourceRecord, UsageAdapter, UsageEventPage,
    UsageFilters,
};
use tokenbuddy_storage::{Database, ImportStats, StorageError};

type SummaryListener = Arc<dyn Fn(QuickSummary) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub database_path: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub claude_home: Option<PathBuf>,
    pub cc_switch_db: Option<PathBuf>,
    pub cockpit_db: Option<PathBuf>,
    pub poll_interval: Duration,
    pub enable_file_watcher: bool,
}

impl CoreConfig {
    pub fn new(database_path: impl Into<PathBuf>, codex_home: Option<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            codex_home,
            claude_home: None,
            cc_switch_db: None,
            cockpit_db: None,
            // Native file notifications are the normal wake-up path. Keep a
            // deliberately infrequent poll as a safety net for filesystems
            // that drop or do not support notifications.
            poll_interval: Duration::from_secs(30),
            enable_file_watcher: true,
        }
    }

    pub fn with_claude_home(mut self, claude_home: Option<PathBuf>) -> Self {
        self.claude_home = claude_home;
        self
    }

    pub fn with_cc_switch_db(mut self, cc_switch_db: Option<PathBuf>) -> Self {
        self.cc_switch_db = cc_switch_db;
        self
    }

    pub fn with_cockpit_db(mut self, cockpit_db: Option<PathBuf>) -> Self {
        self.cockpit_db = cockpit_db;
        self
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("adapter error: {0}")]
    Adapter(String),
    #[error("core lock is poisoned: {0}")]
    Lock(&'static str),
    #[error("failed to start core worker: {0}")]
    Worker(#[from] std::io::Error),
    #[error("core worker did not stop cleanly")]
    WorkerDidNotStop,
    #[error("core worker did not become ready")]
    WorkerNotReady,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub inserted_events: u64,
    pub duplicate_events: u64,
    pub upserted_sessions: u64,
    pub updated_cursors: u64,
    pub skipped_records: usize,
    pub pruned_events: u64,
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

#[derive(Debug, Default)]
struct RefreshState {
    report: ImportReport,
    warnings: Vec<String>,
    has_healthy_source: bool,
    has_error: bool,
}

#[derive(Debug, Clone, Copy)]
enum WorkerSignal {
    Stop,
    Wake,
    FileEvent,
}

struct CoreControl {
    stop: AtomicBool,
    signal: Sender<WorkerSignal>,
}

impl CoreControl {
    fn new(signal: Sender<WorkerSignal>) -> Self {
        Self {
            stop: AtomicBool::new(false),
            signal,
        }
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.signal.send(WorkerSignal::Stop);
    }

    fn wake(&self) {
        let _ = self.signal.send(WorkerSignal::Wake);
    }

    fn signal_sender(&self) -> Sender<WorkerSignal> {
        self.signal.clone()
    }
}

pub struct Core {
    database: Mutex<Database>,
    import_lock: Mutex<()>,
    codex_home: RwLock<Option<PathBuf>>,
    claude_home: RwLock<Option<PathBuf>>,
    cc_switch_db: RwLock<Option<PathBuf>>,
    cockpit_db: RwLock<Option<PathBuf>>,
    summary: RwLock<QuickSummary>,
    summary_listeners: Mutex<Vec<SummaryListener>>,
    control: Arc<CoreControl>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Core {
    pub fn start(config: CoreConfig) -> Result<Arc<Self>, CoreError> {
        let database = Database::open(&config.database_path)?;
        let mut settings = database.get_app_settings()?;
        let mut settings_changed = false;
        if settings.codex_home.is_none() && config.codex_home.is_some() {
            settings.codex_home = config
                .codex_home
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            settings_changed = true;
        }
        if settings.claude_home.is_none() && config.claude_home.is_some() {
            settings.claude_home = config
                .claude_home
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            settings_changed = true;
        }
        if settings_changed {
            database.save_app_settings(&settings)?;
        }
        let codex_home = settings
            .codex_home
            .as_deref()
            .map(PathBuf::from)
            .or(config.codex_home);
        let claude_home = settings
            .claude_home
            .as_deref()
            .map(PathBuf::from)
            .or(config.claude_home);
        let cc_switch_db = settings
            .cc_switch_db_path
            .as_deref()
            .map(PathBuf::from)
            .or(config.cc_switch_db);
        let cockpit_db = settings
            .cockpit_path
            .as_deref()
            .map(PathBuf::from)
            .or(config.cockpit_db);
        let (signal_sender, signal_receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let control = Arc::new(CoreControl::new(signal_sender));
        let core = Arc::new(Self {
            database: Mutex::new(database),
            import_lock: Mutex::new(()),
            codex_home: RwLock::new(codex_home),
            claude_home: RwLock::new(claude_home),
            cc_switch_db: RwLock::new(cc_switch_db),
            cockpit_db: RwLock::new(cockpit_db),
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
        let enable_file_watcher = config.enable_file_watcher;
        let worker = thread::Builder::new()
            .name("tokenbuddy-core".to_owned())
            .spawn(move || {
                worker_loop(
                    weak_core,
                    control,
                    signal_receiver,
                    ready_sender,
                    poll_interval,
                    enable_file_watcher,
                )
            })?;
        core.worker
            .lock()
            .map_err(|_| CoreError::Lock("worker"))?
            .replace(worker);
        // The worker announces readiness before any watcher setup, so this is a
        // generous ceiling for a loaded machine rather than a tight budget.
        ready_receiver
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| CoreError::WorkerNotReady)?;
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

    pub fn rescan_claude(&self, claude_home: Option<PathBuf>) -> Result<ImportReport, CoreError> {
        if let Some(claude_home) = claude_home {
            self.set_claude_home(Some(claude_home))?;
        }
        self.refresh_once()
    }

    pub fn rescan_cc_switch(
        &self,
        cc_switch_db: Option<PathBuf>,
    ) -> Result<ImportReport, CoreError> {
        if let Some(cc_switch_db) = cc_switch_db {
            self.set_cc_switch_db(Some(cc_switch_db))?;
        }
        self.refresh_once()
    }

    pub fn set_cc_switch_db(&self, cc_switch_db: Option<PathBuf>) -> Result<(), CoreError> {
        let mut settings = self.get_app_settings()?;
        settings.cc_switch_db_path = cc_switch_db
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        self.database_lock()?.save_app_settings(&settings)?;
        *self
            .cc_switch_db
            .write()
            .map_err(|_| CoreError::Lock("CC-Switch database"))? = cc_switch_db;
        self.control.wake();
        Ok(())
    }

    pub fn cc_switch_db(&self) -> Result<Option<PathBuf>, CoreError> {
        self.cc_switch_db
            .read()
            .map(|path| path.clone())
            .map_err(|_| CoreError::Lock("CC-Switch database"))
    }

    pub fn detect_cc_switch_path(&self) -> Result<DetectionResult, CoreError> {
        let Some(db_path) = self.cc_switch_db()? else {
            return Ok(DetectionResult {
                source_id: CC_SWITCH_SOURCE_ID.to_owned(),
                detected: false,
                path_or_endpoint: None,
                detected_version: None,
                message: Some("未配置 CC-Switch 数据库".to_owned()),
            });
        };
        CcSwitchAdapter::new(db_path)
            .detect_sync()
            .map_err(|error| CoreError::Adapter(error.to_string()))
    }

    pub fn rescan_cockpit(&self, cockpit_db: Option<PathBuf>) -> Result<ImportReport, CoreError> {
        if let Some(cockpit_db) = cockpit_db {
            self.set_cockpit_db(Some(cockpit_db))?;
        }
        self.refresh_once()
    }

    pub fn set_cockpit_db(&self, cockpit_db: Option<PathBuf>) -> Result<(), CoreError> {
        let mut settings = self.get_app_settings()?;
        settings.cockpit_path = cockpit_db
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        self.database_lock()?.save_app_settings(&settings)?;
        *self
            .cockpit_db
            .write()
            .map_err(|_| CoreError::Lock("Cockpit database"))? = cockpit_db;
        self.control.wake();
        Ok(())
    }

    pub fn cockpit_db(&self) -> Result<Option<PathBuf>, CoreError> {
        self.cockpit_db
            .read()
            .map(|path| path.clone())
            .map_err(|_| CoreError::Lock("Cockpit database"))
    }

    pub fn detect_cockpit_path(&self) -> Result<DetectionResult, CoreError> {
        let Some(db_path) = self.cockpit_db()? else {
            return Ok(DetectionResult {
                source_id: COCKPIT_SOURCE_ID.to_owned(),
                detected: false,
                path_or_endpoint: None,
                detected_version: None,
                message: Some("未配置 Cockpit 数据库".to_owned()),
            });
        };
        CockpitAdapter::new(db_path)
            .detect_sync()
            .map_err(|error| CoreError::Adapter(error.to_string()))
    }

    pub fn set_codex_home(&self, codex_home: Option<PathBuf>) -> Result<(), CoreError> {
        let mut settings = self.get_app_settings()?;
        settings.codex_home = codex_home
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        self.database_lock()?.save_app_settings(&settings)?;
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

    pub fn set_claude_home(&self, claude_home: Option<PathBuf>) -> Result<(), CoreError> {
        let mut settings = self.get_app_settings()?;
        settings.claude_home = claude_home
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        self.database_lock()?.save_app_settings(&settings)?;
        *self
            .claude_home
            .write()
            .map_err(|_| CoreError::Lock("Claude Home"))? = claude_home;
        self.control.wake();
        Ok(())
    }

    pub fn claude_home(&self) -> Result<Option<PathBuf>, CoreError> {
        self.claude_home
            .read()
            .map(|path| path.clone())
            .map_err(|_| CoreError::Lock("Claude Home"))
    }

    pub fn is_running(&self) -> bool {
        !self.control.stop.load(Ordering::SeqCst)
    }

    pub fn get_app_settings(&self) -> Result<AppSettings, CoreError> {
        self.database_lock()?
            .get_app_settings()
            .map_err(CoreError::from)
    }

    pub fn update_app_settings(&self, settings: AppSettings) -> Result<(), CoreError> {
        self.database_lock()?.save_app_settings(&settings)?;
        *self
            .codex_home
            .write()
            .map_err(|_| CoreError::Lock("Codex home"))? = settings.codex_home.map(PathBuf::from);
        *self
            .claude_home
            .write()
            .map_err(|_| CoreError::Lock("Claude Home"))? = settings.claude_home.map(PathBuf::from);
        *self
            .cc_switch_db
            .write()
            .map_err(|_| CoreError::Lock("CC-Switch database"))? =
            settings.cc_switch_db_path.map(PathBuf::from);
        *self
            .cockpit_db
            .write()
            .map_err(|_| CoreError::Lock("Cockpit database"))? =
            settings.cockpit_path.map(PathBuf::from);
        self.control.wake();
        Ok(())
    }

    pub fn list_providers(&self) -> Result<Vec<tokenbuddy_domain::ProviderSummary>, CoreError> {
        self.database_lock()?
            .list_providers()
            .map_err(CoreError::from)
    }

    pub fn list_quota_snapshots(
        &self,
        account_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<tokenbuddy_domain::QuotaSnapshot>, CoreError> {
        self.database_lock()?
            .list_quota_snapshots(account_id, limit)
            .map_err(CoreError::from)
    }

    pub fn detect_codex_path(&self) -> Result<DetectionResult, CoreError> {
        let Some(home) = self.codex_home()? else {
            return Ok(DetectionResult {
                source_id: CODEX_SOURCE_ID.to_owned(),
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

    pub fn detect_claude_path(&self) -> Result<DetectionResult, CoreError> {
        let Some(home) = self.claude_home()? else {
            return Ok(DetectionResult {
                source_id: CLAUDE_SOURCE_ID.to_owned(),
                detected: false,
                path_or_endpoint: None,
                detected_version: None,
                message: Some("未配置 Claude Home".to_owned()),
            });
        };
        ClaudeSessionAdapter::new(home)
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
        self.dashboard_summary_filtered(UsageFilters::default())
    }

    pub fn dashboard_summary_filtered(
        &self,
        mut filters: UsageFilters,
    ) -> Result<DashboardSummary, CoreError> {
        // Anchor "today" on the local calendar day so the dashboard's default
        // window matches the tray tooltip and the user's wall clock, not UTC.
        let period_start = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .map(|start| start.with_timezone(&Utc))
            .ok_or_else(|| CoreError::Adapter("无法计算今日统计窗口".to_owned()))?;
        filters.period_start.get_or_insert(period_start);
        filters
            .period_end
            .get_or_insert(period_start + chrono::Duration::days(1));
        self.database_lock()?
            .dashboard_summary_filtered(&filters)
            .map_err(CoreError::from)
    }

    pub fn list_sessions(
        &self,
        filters: &UsageFilters,
        limit: u64,
        offset: u64,
    ) -> Result<SessionPage, CoreError> {
        self.database_lock()?
            .list_session_page(filters, limit, offset)
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

    pub fn list_usage_events_filtered(
        &self,
        session_id: Option<&str>,
        limit: u64,
        offset: u64,
        filters: &UsageFilters,
    ) -> Result<UsageEventPage, CoreError> {
        self.database_lock()?
            .list_usage_events_filtered(session_id, limit, offset, filters)
            .map_err(CoreError::from)
    }

    pub fn export_usage(
        &self,
        format: &str,
        filters: &UsageFilters,
    ) -> Result<ExportResult, CoreError> {
        self.database_lock()?
            .export_usage(format, filters)
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

    fn watch_paths(&self) -> Result<Vec<PathBuf>, CoreError> {
        let mut paths = Vec::new();
        if let Some(home) = self.codex_home()?
            && let Some(path) = watch_target(&home, "sessions")
        {
            paths.push(path);
        }
        if let Some(home) = self.claude_home()?
            && let Some(path) = watch_target(&home, "projects")
        {
            paths.push(path);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn refresh_once(&self) -> Result<ImportReport, CoreError> {
        let _import_guard = self
            .import_lock
            .lock()
            .map_err(|_| CoreError::Lock("import"))?;
        let mut state = RefreshState::default();

        match self.codex_home()? {
            Some(home) => self.import_codex(home, &mut state)?,
            None => state
                .warnings
                .push("未配置 Codex Home；Codex 数据源保持 Unavailable".to_owned()),
        }
        match self.claude_home()? {
            Some(home) => self.import_claude(home, &mut state)?,
            None => state
                .warnings
                .push("未配置 Claude Home；Claude 数据源保持 Unavailable".to_owned()),
        }

        // CC-Switch and Cockpit are optional and additive: import each only when
        // its database is actually present so users without it see no noise.
        if let Some(db_path) = self.cc_switch_db()? {
            let adapter = CcSwitchAdapter::new(db_path);
            if adapter.db_path().is_file() {
                self.import_cc_switch(adapter, &mut state)?;
            }
        }
        if let Some(db_path) = self.cockpit_db()? {
            let adapter = CockpitAdapter::new(db_path);
            if adapter.db_path().is_file() {
                self.import_cockpit(adapter, &mut state)?;
            }
        }

        // Enforce the retention window after importing so deleted source files,
        // mis-imports, and stale rows do not accumulate forever.
        {
            let mut database = self.database_lock()?;
            let retention_days = database.get_app_settings()?.data_retention_days;
            let outcome = database.enforce_retention(retention_days, Utc::now())?;
            state.report.pruned_events += outcome.deleted_events;
        }

        let warning = (!state.warnings.is_empty()).then(|| state.warnings.join("；"));
        self.refresh_summary(
            if state.has_error {
                CollectionStatus::Error
            } else if state.has_healthy_source {
                CollectionStatus::Collecting
            } else {
                CollectionStatus::Idle
            },
            warning.clone(),
        )?;
        state.report.warning = warning;
        Ok(state.report)
    }

    fn import_codex(&self, codex_home: PathBuf, state: &mut RefreshState) -> Result<(), CoreError> {
        let adapter = CodexSessionAdapter::new(codex_home.clone());
        let cursors = self.database_lock()?.list_import_cursors(adapter.id())?;
        match adapter.import_history_sync(&cursors) {
            Ok(batch) => self.apply_batch("Codex", batch, state),
            Err(error) => {
                let message = format!("Codex 导入失败：{error}");
                state.has_error = true;
                state.warnings.push(message.clone());
                self.record_source_error(
                    CODEX_SOURCE_ID,
                    CODEX_ADAPTER_TYPE,
                    CODEX_DISPLAY_NAME,
                    codex_home,
                    message,
                )
            }
        }
    }

    fn import_claude(
        &self,
        claude_home: PathBuf,
        state: &mut RefreshState,
    ) -> Result<(), CoreError> {
        let adapter = ClaudeSessionAdapter::new(claude_home.clone());
        let cursors = self.database_lock()?.list_import_cursors(adapter.id())?;
        match adapter.import_history_sync(&cursors) {
            Ok(batch) => self.apply_batch("Claude Code", batch, state),
            Err(error) => {
                let message = format!("Claude Code 导入失败：{error}");
                state.has_error = true;
                state.warnings.push(message.clone());
                self.record_source_error(
                    CLAUDE_SOURCE_ID,
                    CLAUDE_ADAPTER_TYPE,
                    CLAUDE_DISPLAY_NAME,
                    claude_home,
                    message,
                )
            }
        }
    }

    fn import_cc_switch(
        &self,
        adapter: CcSwitchAdapter,
        state: &mut RefreshState,
    ) -> Result<(), CoreError> {
        let db_path = adapter.db_path().to_owned();
        let cursors = self
            .database_lock()?
            .list_import_cursors(CC_SWITCH_SOURCE_ID)?;
        match adapter.import_history_sync(&cursors) {
            Ok(batch) => self.apply_batch("CC-Switch", batch, state),
            Err(error) => {
                let message = format!("CC-Switch 导入失败：{error}");
                state.has_error = true;
                state.warnings.push(message.clone());
                self.record_source_error(
                    CC_SWITCH_SOURCE_ID,
                    CC_SWITCH_ADAPTER_TYPE,
                    CC_SWITCH_DISPLAY_NAME,
                    db_path,
                    message,
                )
            }
        }
    }

    fn import_cockpit(
        &self,
        adapter: CockpitAdapter,
        state: &mut RefreshState,
    ) -> Result<(), CoreError> {
        let db_path = adapter.db_path().to_owned();
        let cursors = self
            .database_lock()?
            .list_import_cursors(COCKPIT_SOURCE_ID)?;
        match adapter.import_history_sync(&cursors) {
            Ok(batch) => self.apply_batch("Cockpit", batch, state),
            Err(error) => {
                let message = format!("Cockpit 导入失败：{error}");
                state.has_error = true;
                state.warnings.push(message.clone());
                self.record_source_error(
                    COCKPIT_SOURCE_ID,
                    COCKPIT_ADAPTER_TYPE,
                    COCKPIT_DISPLAY_NAME,
                    db_path,
                    message,
                )
            }
        }
    }

    fn apply_batch(
        &self,
        label: &str,
        batch: ImportBatch,
        state: &mut RefreshState,
    ) -> Result<(), CoreError> {
        let health_status = batch
            .source
            .as_ref()
            .and_then(|source| source.health_status.as_deref());
        if health_status != Some("not_found") {
            state.has_healthy_source = true;
        }
        if batch.skipped_records > 0 {
            state.warnings.push(format!(
                "{label} 导入跳过 {} 条无法解析的记录",
                batch.skipped_records
            ));
        }
        let stats = self.database_lock()?.apply_import_batch(&batch)?;
        state.report.inserted_events += stats.inserted_events;
        state.report.duplicate_events += stats.duplicate_events;
        state.report.upserted_sessions += stats.upserted_sessions;
        state.report.updated_cursors += stats.updated_cursors;
        state.report.skipped_records += batch.skipped_records;
        Ok(())
    }

    fn record_source_error(
        &self,
        source_id: &str,
        adapter_type: &str,
        display_name: &str,
        path: PathBuf,
        error: String,
    ) -> Result<(), CoreError> {
        let timestamp = Utc::now();
        let source = SourceRecord {
            id: source_id.to_owned(),
            adapter_type: adapter_type.to_owned(),
            display_name: display_name.to_owned(),
            path_or_endpoint: Some(path.to_string_lossy().into_owned()),
            enabled: true,
            detected_version: Some("jsonl".to_owned()),
            health_status: Some("error".to_owned()),
            last_success_at: None,
            last_error: Some(error),
            created_at: timestamp,
            updated_at: timestamp,
        };
        self.database_lock()?.apply_import_batch(&ImportBatch {
            source: Some(source),
            ..ImportBatch::default()
        })?;
        Ok(())
    }

    fn refresh_summary(
        &self,
        status: CollectionStatus,
        warning: Option<String>,
    ) -> Result<(), CoreError> {
        let summary = self
            .database_lock()?
            .quick_summary(Utc::now(), status, warning)?;
        let changed = {
            let current = self
                .summary
                .read()
                .map_err(|_| CoreError::Lock("summary"))?;
            *current != summary
        };
        *self
            .summary
            .write()
            .map_err(|_| CoreError::Lock("summary"))? = summary.clone();
        if changed {
            self.notify_summary(summary)?;
        }
        Ok(())
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
            // If Core is being dropped *on* its own worker thread (the worker
            // held the last Arc), joining would be a self-join — a guaranteed
            // deadlock or panic. request_stop above already told the loop to
            // exit and the now-zero strong count makes its next weak upgrade
            // fail, so detaching is safe in that case.
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn worker_loop(
    weak_core: std::sync::Weak<Core>,
    control: Arc<CoreControl>,
    signal_receiver: Receiver<WorkerSignal>,
    ready_sender: Sender<()>,
    poll_interval: Duration,
    enable_file_watcher: bool,
) {
    let mut watchers: Vec<RecommendedWatcher> = Vec::new();
    let mut watched_paths: Vec<PathBuf> = Vec::new();
    let mut ready_sent = false;
    loop {
        if control.stop.load(Ordering::SeqCst) {
            break;
        }

        // Refresh watchers while only *briefly* holding a strong reference, then
        // drop it before blocking on recv below. If the worker held the last Arc
        // across the wait, dropping it there would run Core::drop on this very
        // thread and deadlock on a self-join; scoping the strong ref avoids it.
        if enable_file_watcher {
            let Some(core) = weak_core.upgrade() else {
                break;
            };
            let desired_paths = core.watch_paths().unwrap_or_default();
            if desired_paths != watched_paths {
                drop(watchers);
                watchers = Vec::new();
                for path in &desired_paths {
                    if let Ok(next_watcher) = create_watcher(&control, path) {
                        watchers.push(next_watcher);
                    }
                }
                watched_paths = desired_paths;
            }
        }

        // Announce readiness only after the first watcher pass so a file event
        // that arrives right after Core::start returns is not missed. The 10s
        // start timeout absorbs a slow first registration.
        if !ready_sent {
            let _ = ready_sender.send(());
            ready_sent = true;
        }

        let signal = match signal_receiver.recv_timeout(poll_interval) {
            Ok(signal) => signal,
            // The timeout is intentional: it is the fallback path for
            // filesystems that do not deliver native notifications.
            Err(mpsc::RecvTimeoutError::Timeout) => WorkerSignal::Wake,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(signal, WorkerSignal::Stop) || control.stop.load(Ordering::SeqCst) {
            break;
        }
        if matches!(signal, WorkerSignal::FileEvent)
            && !coalesce_file_events(&signal_receiver, &control)
        {
            break;
        }
        while signal_receiver.try_recv().is_ok() {}

        let Some(core) = weak_core.upgrade() else {
            break;
        };
        if let Err(error) = core.refresh_once() {
            let _ = core.refresh_summary(CollectionStatus::Error, Some(error.to_string()));
        }
    }
}

fn watch_target(home: &std::path::Path, child: &str) -> Option<PathBuf> {
    let data_dir = home.join(child);
    if data_dir.is_dir() {
        return Some(data_dir);
    }
    // The data directory does not exist yet. Watch the (small, app-owned) home
    // directory so its creation wakes the importer — but never fall back to the
    // user's entire HOME or a filesystem root. A RecursiveMode::Recursive watch
    // on those would traverse the whole tree, exhausting inotify handles on
    // Linux and stalling startup. When home is overbroad or absent, skip the
    // watch entirely and let the periodic poll pick the directory up once it
    // appears.
    if home.is_dir() && !is_overbroad_watch_root(home) {
        return Some(home.to_owned());
    }
    None
}

fn is_overbroad_watch_root(path: &std::path::Path) -> bool {
    // A filesystem root has no parent.
    if path.parent().is_none() {
        return true;
    }
    // A codex/claude home misconfigured to the user's HOME directory itself.
    home_dir().is_some_and(|home| home == path)
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn coalesce_file_events(receiver: &Receiver<WorkerSignal>, control: &CoreControl) -> bool {
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        if control.stop.load(Ordering::SeqCst) {
            return false;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return true;
        };
        match receiver.recv_timeout(remaining) {
            Ok(WorkerSignal::Stop) => return false,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn create_watcher(
    control: &CoreControl,
    path: &std::path::Path,
) -> notify::Result<RecommendedWatcher> {
    let signal = control.signal_sender();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if result.is_ok() {
            let _ = signal.send(WorkerSignal::FileEvent);
        }
    })?;
    let mode = if path.is_dir() {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(path, mode)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        path::Path,
        thread,
        time::{Duration, Instant},
    };

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

    fn append_usage(path: &Path, request_id: &str) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open fixture");
        writeln!(
            file,
            "{{\"type\":\"response.completed\",\"session_id\":\"simple-session\",\"timestamp\":\"{}\",\"request_id\":\"{}\",\"model\":\"gpt-5-codex\",\"usage\":{{\"input_tokens\":20,\"output_tokens\":8}}}}",
            Utc::now().to_rfc3339(),
            request_id,
        )
        .expect("append fixture record");
    }

    fn wait_for_events(core: &Core, total: u64, timeout: Duration) {
        let started = Instant::now();
        loop {
            if core
                .list_usage_events(None, 100, 0)
                .expect("read events")
                .total
                == total
            {
                return;
            }
            assert!(started.elapsed() < timeout, "event import timed out");
            thread::sleep(Duration::from_millis(15));
        }
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
        assert!(summary.active_session_id.is_some());
        assert_eq!(summary.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(summary.session_output_tokens, Some(78));
        assert_eq!(summary.today_total_tokens, Some(28));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn native_file_events_refresh_before_the_polling_fallback() {
        let (home, session_path) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let mut config = CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        );
        config.poll_interval = Duration::from_secs(5);
        let core = Core::start(config).expect("core starts");
        assert_eq!(
            core.list_usage_events(None, 100, 0).expect("events").total,
            2
        );

        append_usage(&session_path, "native-notify-request");
        wait_for_events(&core, 3, Duration::from_secs(2));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn polling_fallback_can_import_when_native_watching_is_disabled() {
        let (home, session_path) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let mut config = CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        );
        config.poll_interval = Duration::from_millis(10);
        config.enable_file_watcher = false;
        let core = Core::start(config).expect("core starts");

        append_usage(&session_path, "polling-fallback-request");
        wait_for_events(&core, 3, Duration::from_secs(2));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn lifecycle_and_shared_entry_handles_use_one_core_until_explicit_exit() {
        let (home, _) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        ))
        .expect("core starts");
        let tray_entry = std::sync::Arc::clone(&core);
        let desktop_entry = std::sync::Arc::clone(&core);
        let web_entry = std::sync::Arc::clone(&core);

        assert!(core.is_running());
        assert!(std::sync::Arc::ptr_eq(&tray_entry, &desktop_entry));
        assert_eq!(
            tray_entry.quick_summary().expect("tray summary"),
            web_entry.quick_summary().expect("web summary")
        );
        assert_eq!(
            desktop_entry
                .list_usage_events(None, 100, 0)
                .expect("desktop events")
                .total,
            2
        );

        core.shutdown().expect("core stops");
        assert!(!core.is_running());
        drop(tray_entry);
        drop(desktop_entry);
        drop(web_entry);
    }

    #[test]
    fn quick_summary_query_p95_stays_within_tray_budget() {
        let (home, _) = fixture_home("simple_session.jsonl");
        let database = tempfile::tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            Some(home.path().to_owned()),
        ))
        .expect("core starts");
        let mut samples = (0..100)
            .map(|_| {
                let started = Instant::now();
                core.quick_summary().expect("quick summary");
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        let p95 = samples[((samples.len() * 95).div_ceil(100)).saturating_sub(1)];
        println!("QuickSummary P95: {} ms", p95.as_secs_f64() * 1_000.0);
        assert!(p95 < Duration::from_millis(50));
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
