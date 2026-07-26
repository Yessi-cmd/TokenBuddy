# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required reading before changing code

- `AGENTS.md` holds the binding project, data-correctness, privacy, and verification rules. They override any default behaviour.
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md` is the canonical product/architecture/acceptance spec (Chinese). Code comments cite it by section (`spec §6.1`, `§11.3`, …). Its final section (`# 34. 实施状态`) tracks which phase is done.
- Every coherent change batch needs a change log at `docs/change-logs/YYYY-MM-DD-NN-brief-topic.md` stating purpose, affected files, behaviour changes, verification performed, and remaining limitations — and the spec's implementation status must be updated when a milestone completes.

## Commands

```sh
pnpm install
pnpm dev              # tauri dev (desktop shell + vite)
pnpm build            # tauri bundle
pnpm build:web        # tsc -b && vite build (frontend only)

pnpm format:check     # prettier + cargo fmt --check
pnpm lint             # eslint --max-warnings 0 + cargo clippy -D warnings
pnpm test             # vitest run + cargo test --workspace --all-targets
pnpm check:rust       # cargo check --workspace --all-targets
```

CI (macOS + Windows) runs exactly those five verification commands plus `tauri build --debug --no-bundle`. Run them locally before declaring a batch done; record the exact commands in the change log.

Single tests:

```sh
cargo test -p tokenbuddy-storage repeated_import        # one Rust test by name
cargo test -p tokenbuddy-core --test phase3_claude      # one integration test file
pnpm --filter @tokenbuddy/desktop test src/App.test.tsx # one vitest file
pnpm --filter @tokenbuddy/desktop exec vitest run -t "renders"
```

`TOKENBUDDY_DEBUG_SHOW_WINDOWS=1` (debug builds only) makes the main window visible at startup instead of tray-only. `TOKENBUDDY_WEB_ROOT` overrides where the loopback server serves the SPA build from.

## Architecture

Rust workspace + one React SPA, wired as strict layers (`Cargo.toml` members):

```
crates/domain    – source-agnostic types + the UsageAdapter trait. No Tauri, no SQLite.
crates/storage   – SQLite (rusqlite, bundled). Migrations, idempotent batch import, all aggregation SQL.
crates/adapters/{codex-session,claude-session,cc-switch,cockpit}
                 – read-only source parsers; each produces an ImportBatch of domain types.
crates/core      – the single long-lived Core: owns the DB connection, runs the importer worker,
                   maintains QuickSummary, exposes every query the UI can make.
apps/desktop/src-tauri – Tauri shell: tray, windows, #[tauri::command] wrappers, loopback HTTP server.
apps/desktop/src       – React 19 SPA (TanStack Query, zustand, echarts).
```

Dependencies point strictly downward. Adapters depend only on `domain`; `storage` depends only on `domain`; the Tauri shell holds an `Arc<Core>` and never touches SQLite or source files itself.

### One Core, several entry points

`Core::start` opens SQLite, does one synchronous import so the tray is useful immediately, then spawns the `tokenbuddy-core` worker thread. The worker wakes on `notify` filesystem events (coalesced over 100 ms) with a 30 s poll as the fallback for filesystems that drop notifications. `Core` is held as an `Arc` and shared by *all* surfaces — tray tooltip, the hidden `main` window, the `/quick` popover window, and the on-demand loopback HTTP server. Adding a second entry point must never mean a second scan, a second import, or different aggregates (`crates/core/tests/phase4b_lifecycle.rs` guards this).

`Core::drop` must not self-join its own worker; the worker only holds a `Weak<Core>` for the same reason. Keep that shape when editing the worker loop.

### Two transports, one contract

The SPA reaches the same data two ways and the TS types in `apps/desktop/src/lib/api.ts` must mirror the Rust domain structs field-for-field (serde uses snake_case, so does the TS):

- Desktop: `invoke("get_dashboard_summary", …)` — commands registered in `apps/desktop/src-tauri/src/lib.rs`.
- Browser: `GET/POST /api/*` on the loopback server in `apps/desktop/src-tauri/src/web.rs`.

`request()` in `api.ts` picks the transport by sniffing `__TAURI_INTERNALS__`. **Any new query must be added in three places: a `Core` method, a `#[tauri::command]`, and an `/api/*` route** — otherwise the web panel silently loses a feature. The server binds `127.0.0.1` *and* `::1` on an ephemeral port only; never `0.0.0.0`.

The SPA is path-routed by `usePathname()` (no router library): `/quick` renders the tray popover, everything else renders the full panel shell (`/dashboard`, `/sessions`, `/providers`, `/quotas`, `/sources`, `/settings`, `/sessions/:id`).

### Tray-first windowing

The app starts as a menu-bar/tray accessory with windows hidden. Left click toggles the frameless translucent `quick` window positioned against the tray rect; double click opens `main`. Closing a window only hides it — only the tray "退出" item (or `quit_tokenbuddy`) stops the Core and exits. Windows are created lazily via `get_or_create_window`.

## Invariants that tests and reviewers enforce

These are not style preferences; violating them corrupts the product's core claim.

- **Missing means missing.** Token counts, costs, quotas, and attribution are `Option`/`null` end to end. Never substitute `0`. Aggregates return `Unavailable` rather than passing off a partial sum as a total (see `UsageTotals`, `NormalizedUsage`).
- **Precision is carried, not assumed.** Every event stores `precision_token/session/provider/account` (`Verified > ExactSession > Correlated > Estimated > Unavailable`) and the UI shows it. Source priority: provider-reported > OTel > session log > proxy log > tokenizer estimate.
- **Token sources vs attribution sources.** Only the Codex and Claude session adapters emit `usage_events`. CC-Switch and Cockpit deliberately emit **none** — they proxy the very requests the session logs already record, so importing their rows would double-count. Their contribution is `ProviderRecord` and `SessionProviderAttribution` (who really served the request; a model name cannot tell you when a relay is in front). Keep that boundary when extending them.
- **Imports are incremental and idempotent.** Each event carries a stable `raw_event_hash`; re-importing a fixture must not change event counts or session aggregates. `ImportCursor` tracks byte offset, file signature (rotation/truncation), the last cumulative usage snapshot, and `last_session_id` (Codex writes the session UUID only in the header line). Cumulative snapshots are differenced with `checked_delta`, which yields `None` rather than a negative delta.
- **Read-only third parties.** CC-Switch and Cockpit databases are opened with `OpenFlags` read-only and `sqlite_master` is probed before any table is touched. Never write to them, their settings, or their credentials.
- **No content, no secrets.** Parsers never copy prompt/completion/source text, headers, keys, or tokens into the domain model or `raw_usage_json`.
- **"Today" is the local calendar day**, computed once in `storage`/`core` so the tray tooltip and the dashboard agree with the wall clock — not UTC.
- **A failing adapter degrades only itself.** `refresh_once` records the error on that source and continues with the others.
- Cross-platform paths only; Windows compiles in CI. `home_dir()` and the default-path helpers already branch on `USERPROFILE` vs `HOME`.

## Conventions

- Storage migrations are append-only: add `crates/storage/migrations/000N_*.sql` and register it in `crates/storage/src/migrations.rs`; `run()` hard-fails if `user_version` doesn't reach the last entry.
- Parser work starts from sanitized fixtures in `fixtures/{codex,claude}/`. Never use real Codex/Claude/CC-Switch/Cockpit data, and never edit an existing fixture to make a regression pass — add a new fixture for a new schema variant.
- Rust: edition 2024, 4-space indent, workspace-pinned dependency versions in the root `Cargo.toml`. TS/JSON/CSS: 2-space, LF, Prettier.
- User-facing strings, tray labels, and error messages are Simplified Chinese; code, identifiers, and doc comments are English.
- Work on `main`; temporary branches only when isolation is genuinely needed, then merge and delete.
