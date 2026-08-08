-- Remember where OpenCode's database lives so the adapter can be configured
-- once and resumed across launches, like the other user-chosen source paths.
ALTER TABLE app_settings ADD COLUMN opencode_db_path TEXT;
