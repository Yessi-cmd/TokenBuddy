# Phase 4b correctness and MVP completion

## Purpose

Fix the high-priority correctness and product gaps found during the implementation audit while keeping the existing sanitized-fixture workflow and shared Core boundary.

## Affected files

- `Cargo.toml`, `Cargo.lock`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/capabilities/default.json`
- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/web.rs`
- `apps/desktop/src/App.tsx`, `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`, `apps/desktop/src/styles.css`
- `crates/domain/src/lib.rs`
- `crates/core/src/lib.rs`
- `crates/storage/src/lib.rs`
- `crates/adapters/codex-session/src/lib.rs`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- QuickSummary now comes only from Core and includes the active session title, project path, Provider and model; it no longer loads or filters a session list.
- Codex import cursors stop before an incomplete final JSONL record and retry that record after it is completed.
- Session, Provider, Dashboard and QuickSummary aggregates preserve `None` when any event in the aggregate lacks the field being summed.
- Dashboard, usage-event and loopback queries share date/app/provider/account/model/project/precision/search filters.
- CSV and JSON exports contain normalized usage and attribution metadata without the raw usage payload.
- Tauri single-instance forwarding, OS autostart synchronization, and loopback settings synchronization are enabled.
- The tray-first app no longer pre-creates the main and Quick WebViews; each is created on first use and then hidden instead of closed.

## Verification

- `pnpm format:check`
- `pnpm lint`
- `pnpm test` — frontend 7 tests; Rust workspace tests all passed (adapter 9, Core 6, lifecycle integration 1, desktop 13, domain 3, storage 4).
- `pnpm build:web`
- `pnpm check:rust`
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`
- `git diff --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Remaining limitations

- Windows real-machine tray, autostart, installer and hidden-window acceptance still need to run in Windows or CI; the local Windows GNU target check is blocked by the missing `x86_64-w64-mingw32-gcc` toolchain.
- Claude Session, OTel, CC Switch, Cockpit, official quota adapters and the optional proxy remain future phases.
- No real user Codex, Claude Code, CC Switch or Cockpit data was used or modified in this batch.
