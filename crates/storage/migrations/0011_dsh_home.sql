-- Remember where DeepSeek Harness keeps its home so the adapter can be
-- configured once and resumed across launches, like the other user-chosen
-- source paths.
ALTER TABLE app_settings ADD COLUMN dsh_home TEXT;
