-- Claude Code may write a provisional zero-usage row before the complete
-- usage object for the same response id. Re-read native Claude transcripts so
-- storage can reconcile those stable identities in place without adding rows.
DELETE FROM import_cursors
 WHERE source_id = 'claude-code-session';
