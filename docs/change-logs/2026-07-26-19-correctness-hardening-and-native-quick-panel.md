# Correctness hardening, feature completion, and a native tray Quick panel

## Purpose

Fix the correctness, security, and lifecycle defects surfaced by an adversarial
multi-dimension review; complete the settings/provider surfaces the spec
promised but never wired up; and restyle the tray Quick panel to read like a
native macOS menu-bar popover.

## Affected files

- `Cargo.toml` (tauri `macos-private-api` feature)
- `apps/desktop/src-tauri/tauri.conf.json` (`macOSPrivateApi`)
- `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/web.rs`
- `apps/desktop/src/App.tsx`, `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`, `apps/desktop/src/styles.css`
- `crates/domain/src/lib.rs`
- `crates/core/src/lib.rs`
- `crates/storage/src/lib.rs`, `crates/storage/src/migrations.rs`
- `crates/storage/migrations/0003_cursor_session_identity.sql` (new)
- `crates/adapters/claude-session/src/lib.rs`
- `crates/adapters/codex-session/src/lib.rs`
- `crates/core/tests/phase3_claude.rs`

## Correctness fixes

- **Event dedup** now keys on the stable request/response identity
  (`message.id` + `requestId`) instead of folding timestamp and raw usage into
  the hash. The same API response written across multiple JSONL lines, or copied
  into a new session file on resume/continue, is counted exactly once
  (spec §16.2).
- **`started_at`** uses `MIN`/`ended_at` uses `MAX` on session upsert, so an
  incremental tail import no longer drags a session's start time forward on every
  poll.
- **Codex session identity** is persisted in the import cursor
  (`last_session_id`, migration `0003`) and restored on incremental import, so
  header-less `token_count` rows stay attached to their session instead of
  splitting off under the file-stem fallback.
- **"今日" boundary** is computed in the local timezone across storage, core, and
  the dashboard date picker, instead of UTC.
- **Claude sidechains** are detected from the real `isSidechain` field; an
  unlabeled sidechain turn with its own session id is attributed to the main
  chain that spawned it.

## Security / lifecycle fixes

- **Static file serving** (`web.rs`) rejects absolute paths and `..`/prefix
  components and canonicalizes within the web root, closing the
  `GET /%2Fetc%2Fpasswd` arbitrary-read hole.
- **File watcher** never falls back to watching `$HOME` or a filesystem root;
  the worker drops its strong `Core` reference before blocking, and `Core::drop`
  avoids a self-join deadlock when dropped on its own worker thread. The startup
  readiness timeout was widened to 10s.

## Feature completion

- **Data retention** is enforced: `enforce_retention` deletes usage, quota
  snapshots, and orphan sessions older than `data_retention_days` after each
  refresh — previously a dead setting.
- **Providers/accounts** are derived from the model + app of imported events, so
  the Providers view reflects real usage instead of staying empty. Provider
  identities already resolved by an adapter are respected.
- **Session list** honors the same date/app/provider/account/model/precision/
  search filters as the dashboard metric cards.
- **Export** on desktop writes the file to the downloads directory via a
  `save_export` command (WKWebView cannot trigger a blob download); the browser
  build keeps the blob fallback.
- Front-end error handling no longer swallows failures: every `catch` logs the
  cause and shows a real message, and scanning reports per-source results and
  parser warnings instead of always blaming Codex.

## Native Quick panel

- The tray Quick window is transparent with a system material (macOS Popover /
  Windows Acrylic) so it reads as a native menu-bar popover.
- The panel is rebuilt as an AppKit-style menu: grouped rows with tinted glyph
  tiles, hairline separators, gray section headers, hover highlights, and footer
  actions ("打开完整面板…", "打开本地网页面板…") backed by a new
  `show_main_window` command.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets` — all suites pass (adapters 11 + 10,
  core 6, integration/lifecycle, domain 3, storage 16, desktop 16).
- `cargo build -p tokenbuddy-desktop`
- `prettier --check .`, `eslint . --max-warnings 0`
- `vitest run` — 8 front-end tests pass
- `tsc -b && vite build`
