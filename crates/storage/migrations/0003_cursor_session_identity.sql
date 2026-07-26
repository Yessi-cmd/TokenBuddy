-- Remember the session identity in force at the cursor's byte offset so an
-- incremental import that resumes past a Codex rollout header keeps appended
-- usage rows attached to the same session instead of splitting them off under
-- the file-stem fallback identity.
ALTER TABLE import_cursors ADD COLUMN last_session_id TEXT;
