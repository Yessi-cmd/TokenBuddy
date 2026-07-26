# CC-Switch and Cockpit Tools read-only adapters

## Purpose

Adapt the two third-party tools the spec reserves (§10 CC-Switch, §11 Cockpit
Tools) as read-only sources, grounded in each tool's **real** on-disk schema
rather than an invented one, so they add real provider/cost/request context
without becoming self-certifying "zombie" features.

## Affected files

- `Cargo.toml`, `Cargo.lock` (two new workspace members)
- `crates/adapters/cc-switch/` (new crate)
- `crates/adapters/cockpit/` (new crate)
- `crates/domain/src/lib.rs` (`ProviderRecord`, `ImportBatch.providers`)
- `crates/storage/src/lib.rs` (`upsert_provider_record`)
- `crates/core/{Cargo.toml,src/lib.rs}` (import + detect + rescan wiring)
- `apps/desktop/src-tauri/{Cargo.toml,src/lib.rs,src/web.rs}` (commands + routes)
- `apps/desktop/src/{App.tsx,App.test.tsx,lib/api.ts}` (source-bar + scan)

## CC-Switch (`crates/adapters/cc-switch`)

- Opens `~/.cc-switch/cc-switch.db` strictly read-only
  (`SQLITE_OPEN_READ_ONLY`), probes `sqlite_master`, and tolerates missing
  tables/columns via `PRAGMA table_info`.
- Imports `proxy_request_logs` rows CC-Switch measured **through its own proxy**
  (`data_source = 'proxy'`) as usage events, keyed on `request_id`, carrying real
  `total_cost_usd`, latency, status, and model.
- **Rows CC-Switch re-derived from `~/.codex`/`~/.claude` session logs
  (`data_source` of `codex_session`/`session_log`) are skipped** so they do not
  double-count TokenBuddy's own Codex/Claude adapters.
- Imports `providers` + `provider_endpoints` into the Providers view (real
  names + upstream URLs).
- Validated against the real 7.8 MB database on the author's machine: 1,731
  proxy events with cost, 2 providers, 0 skipped (the ~11.7k session-derived
  rows correctly excluded). Reproduce with the `#[ignore]` `real_database` test
  and `CC_SWITCH_REAL_DB`.

## Cockpit Tools (`crates/adapters/cockpit`)

- Cockpit Tools = the open-source `jlcodes99/cockpit-tools`. Per spec §11.1 the
  integration prefers a public interface over reverse-engineering the
  credentials store; the credential/auth files are never read.
- Uses the non-sensitive read-only SQLite `request_logs` table in
  `~/.antigravity_cockpit/codex_local_access_logs.sqlite` — the only stable,
  request-level, non-secret surface. Rows are keyed on the unique `event_key`
  and carry tokens, `estimated_cost_usd`, status, model, and `account_id`.
- Grounded in the real table schema read from disk; validated to parse the real
  database without panic (currently 0 rows — the table only fills when Codex is
  actually routed through Cockpit's local proxy). Reproduce with the `#[ignore]`
  `real_database` test and `COCKPIT_REAL_DB`.

## Wiring

- `CoreConfig` gains `cc_switch_db`/`cockpit_db`; both default to the standard
  path but only import when the file is present (no noise for users without the
  tool). Sourced from the existing `cc_switch_db_path`/`cockpit_path` settings.
- Both are imported every refresh (incremental via a timestamp cursor; request
  identity dedupes re-reads) and enforce the retention window like other sources.
- New Tauri commands `detect_cc_switch_path`/`rescan_cc_switch`/
  `detect_cockpit_path`/`rescan_cockpit`, matching loopback routes, api.ts
  wrappers, and dashboard source-bar detect buttons. The scan button now reads
  "扫描全部来源" and reports per-source results.

## Known follow-ups (documented, not silently missing)

- Cockpit account **alias/plan** (WebSocket `request.get_accounts`) and
  **hourly/weekly quota** (`/report` HTTP API, disabled by default) are separate
  channels not yet consumed; per spec §11.3 Cockpit usage is marked
  `Correlated`/`ExactSession`, never `Verified`.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets` — 11 suites pass (adds cc-switch 4 +
  cockpit 4 fixture tests; 2 `#[ignore]` real-DB smoke tests)
- `cargo build -p tokenbuddy-desktop`
- `prettier --check .`, `eslint . --max-warnings 0`, `vitest run` (8), `vite build`
