# Complete Phase 1 domain and storage core

## Purpose

Implement the shared data model and SQLite foundation required by the first
TokenBuddy import pipeline, with explicit missing-value and precision semantics.

## Affected files

- `Cargo.toml`
- `Cargo.lock`
- `crates/domain/Cargo.toml`
- `crates/domain/src/lib.rs`
- `crates/storage/Cargo.toml`
- `crates/storage/src/lib.rs`
- `crates/storage/src/migrations.rs`
- `crates/storage/migrations/0001_initial.sql`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- Added source-independent domain types for applications, launchers, ingest
  sources, precision levels, normalized usage, sessions, cursors, adapters,
  dashboard summaries, and usage event pages.
- Added SQLite migrations for sources, providers, accounts, sessions,
  usage_events, quota_snapshots, and import_cursors, including indexes and
  persistent cumulative-snapshot state.
- Added transactional batch import with stable event-hash idempotency,
  source/session upserts, cursor persistence, dashboard aggregation, session
  summaries, session detail, and paginated usage-event queries.
- Preserved unknown token and cost values as `NULL`/`None`; cache hit rate is
  unavailable unless its required operands are known and valid.

## Verification performed

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets` — 6 Rust tests passed
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Remaining limitations

- The Codex adapter and sanitized fixtures are implemented in the next phase.
- File watching, OTel, third-party adapters, and the optional local proxy are
  not part of this batch.
