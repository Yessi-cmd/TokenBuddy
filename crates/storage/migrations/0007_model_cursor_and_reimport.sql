-- Persist the model context needed to resume Codex rollout files after the
-- header. Token-count rows commonly omit it, just like they omit session id.
ALTER TABLE import_cursors ADD COLUMN last_model TEXT;

-- The model enrichment and pricing rules are parser semantics, not a schema
-- change visible in the source files. Re-read existing native session logs so
-- rows imported by an older build can be reconciled in place without changing
-- their stable event hashes or counting them a second time.
DELETE FROM import_cursors
 WHERE source_id IN ('codex-session', 'claude-code-session');
