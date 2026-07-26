-- Periods during which a launcher routed requests for one account.
--
-- Codex and Claude session logs never record an account, and `auth.json` only
-- names the account signed in right now, so a machine that rotates several
-- accounts through one home has no way to attribute history from those sources
-- alone. A launcher that proxied the requests (Cockpit, CC-Switch) does know,
-- and its request log yields time ranges — enough to correlate a usage event by
-- its timestamp (spec §17.2). Kept in its own table so the attribution applies
-- to events imported in any order, exactly like session_provider_attributions.
CREATE TABLE IF NOT EXISTS account_activity_windows (
    account_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    app TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, source_id, started_at)
);

CREATE INDEX IF NOT EXISTS idx_account_windows_lookup
    ON account_activity_windows (app, started_at, ended_at);
