# Phase 4b product acceptance

## Purpose

Complete the Phase 4b implementation needed for a single Core to serve the tray, desktop, quick-summary, and local-web entry points consistently, while reducing idle work and exposing the remaining shared SPA views.

## Affected files

- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `Cargo.toml`
- `Cargo.lock`
- `crates/core/Cargo.toml`
- `crates/core/src/lib.rs`
- `crates/core/tests/phase4b_lifecycle.rs`
- `crates/domain/src/lib.rs`
- `crates/storage/src/lib.rs`
- `crates/storage/src/migrations.rs`
- `crates/storage/migrations/0002_app_settings.sql`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/web.rs`
- `apps/desktop/src/App.tsx`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/styles.css`

## Behaviour changes

- `tokenbuddy-core` now owns a `notify` watcher for the Codex sessions directory, coalesces burst events, and retains a low-frequency polling fallback for filesystems that do not deliver native notifications.
- Repeated refreshes update the cached `QuickSummary` without notifying the tray when the summary is unchanged; the default fallback interval is 30 seconds instead of a 2-second full scan loop.
- Core settings, provider summaries, and quota snapshots are persisted/queryable through the storage and Tauri/loopback service layers. Unknown values stay `Unavailable`/`None`.
- The shared React SPA now routes `/providers`, `/quotas`, `/settings`, `/sources`, `/sessions`, and `/sessions/:id` through the same query API used by the desktop and local web entries.
- The local web server binds both `127.0.0.1` and `[::1]` on the same ephemeral port and serves the new read/write settings and provider/quota routes.
- A sanitized integration fixture verifies that tray, desktop, and web handles share one Core, see the same native-event import, and stop together on explicit shutdown.

## Verification

- `pnpm format:check` passed.
- `pnpm lint` passed, including `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- `pnpm test` passed: 6 frontend tests, 7 Core tests including 1 integration test, 8 desktop/web tests, 5 Codex adapter tests, 3 domain tests, and 3 storage tests.
- `pnpm build:web` passed.
- `pnpm check:rust` passed.
- `pnpm --filter @tokenbuddy/desktop tauri build --debug` passed and produced the macOS `.app` and `.dmg` bundles.
- Core `QuickSummary` P95: `0.000209 ms` (budget `<50 ms`).
- Loopback HTTP `QuickSummary` P95: `6.388625 ms` (budget `<200 ms`); the test also served identical responses through IPv4 and IPv6 loopback.
- Packaged macOS idle sampling after startup: 40 samples at 250 ms, average `0.02%` CPU, P95 `0.10%`, maximum `0.50%`; the process remained resident with the Core worker running while the windows were hidden.
- `git diff --check` passed.

## Remaining limitations

- This host cannot provide a Windows desktop session, so Windows tray click/double-click/right-click behaviour and Windows CPU/P95 remain covered by the cross-platform source/CI configuration but are not real-machine observations in this batch.
- macOS Computer Use timed out while inspecting the hidden accessory/menu-bar UI; packaged process liveness and Core worker sampling were verified, but direct menu-bar click assertions were not possible through the available accessibility bridge.
- Claude Session, OTel, CC Switch/Cockpit adapters, official quota ingestion, and the optional proxy remain future phases. No external provider pricing or quota tokens are inferred.
