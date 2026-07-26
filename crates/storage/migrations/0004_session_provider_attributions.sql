-- Real provider/account identity for a session, reported by the launcher that
-- proxied the requests (CC-Switch, Cockpit). Session logs know the model but
-- never the upstream, so without this the provider is guessed from the model
-- name — wrong whenever a relay sits in front. Kept in its own table so the
-- attribution survives and applies to events imported later, in any order.
CREATE TABLE IF NOT EXISTS session_provider_attributions (
    session_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    account_id TEXT,
    source_id TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
