-- Per-install random salt used to fingerprint account identities and API keys
-- (spec §20.2: fingerprint = SHA256(local_random_salt + secret)). It lives next
-- to the settings row but deliberately outside `AppSettings`, so it is never
-- serialized to the UI, the loopback API, or an export.
ALTER TABLE app_settings ADD COLUMN local_salt TEXT;
