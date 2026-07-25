CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    adapter_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    path_or_endpoint TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    detected_version TEXT,
    health_status TEXT,
    last_success_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    provider_family TEXT NOT NULL,
    display_name TEXT NOT NULL,
    upstream_url TEXT,
    launcher TEXT,
    source_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    display_name TEXT,
    account_fingerprint TEXT NOT NULL,
    auth_mode TEXT NOT NULL,
    plan TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    external_session_id TEXT,
    parent_session_id TEXT,
    app TEXT NOT NULL,
    launcher TEXT,
    project_path TEXT,
    title TEXT,
    started_at TEXT,
    ended_at TEXT,
    source_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_events (
    id TEXT PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    app TEXT NOT NULL,
    launcher TEXT,
    ingest_source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    provider_id TEXT,
    account_id TEXT,
    session_id TEXT,
    parent_session_id TEXT,
    request_id TEXT,
    response_id TEXT,
    model TEXT,
    query_source TEXT,
    input_tokens_total INTEGER,
    input_tokens_uncached INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    output_tokens_total INTEGER,
    reasoning_tokens INTEGER,
    visible_output_tokens INTEGER,
    provider_reported_cost REAL,
    estimated_cost REAL,
    currency TEXT,
    http_status INTEGER,
    latency_ms INTEGER,
    success INTEGER,
    precision_token TEXT NOT NULL,
    precision_session TEXT NOT NULL,
    precision_provider TEXT NOT NULL,
    precision_account TEXT NOT NULL,
    raw_event_hash TEXT NOT NULL UNIQUE,
    raw_usage_json TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS quota_snapshots (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    window_type TEXT NOT NULL,
    used_percent REAL,
    remaining_percent REAL,
    reset_at TEXT,
    credits_remaining REAL,
    precision TEXT NOT NULL,
    raw_json TEXT
);

CREATE TABLE IF NOT EXISTS import_cursors (
    source_id TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    file_size INTEGER,
    modified_at TEXT,
    byte_offset INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT,
    last_cumulative_usage TEXT,
    snapshot_generation INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (source_id, resource_id)
);

CREATE INDEX IF NOT EXISTS idx_usage_time ON usage_events(occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_events(session_id);
CREATE INDEX IF NOT EXISTS idx_usage_provider ON usage_events(provider_id);
CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_events(model);
CREATE INDEX IF NOT EXISTS idx_usage_app ON usage_events(app);
CREATE INDEX IF NOT EXISTS idx_usage_request ON usage_events(request_id);
CREATE INDEX IF NOT EXISTS idx_quota_account_time ON quota_snapshots(account_id, captured_at);
