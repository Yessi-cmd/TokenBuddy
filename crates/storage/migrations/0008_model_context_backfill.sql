-- The parser now reconciles usage rows when a response names the model after
-- the token snapshot. Re-read native session logs once so existing databases
-- receive that same-session backfill without changing event identity.
DELETE FROM import_cursors
 WHERE source_id IN ('codex-session', 'claude-code-session');
