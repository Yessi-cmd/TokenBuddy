CREATE TABLE IF NOT EXISTS app_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    codex_home TEXT,
    claude_home TEXT,
    cc_switch_db_path TEXT,
    cockpit_path TEXT,
    otel_port INTEGER,
    auto_start INTEGER NOT NULL DEFAULT 0,
    proxy_enabled INTEGER NOT NULL DEFAULT 0,
    save_request_metadata INTEGER NOT NULL DEFAULT 0,
    data_retention_days INTEGER
);
