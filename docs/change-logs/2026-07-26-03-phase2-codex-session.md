# Complete Phase 2 Codex session import

## Purpose

Implement the first read-only Codex Session adapter using sanitized JSONL
fixtures, with incremental cursors and safe handling of cumulative snapshots.

## Affected files

- `Cargo.toml`
- `Cargo.lock`
- `crates/adapters/codex-session/Cargo.toml`
- `crates/adapters/codex-session/src/lib.rs`
- `fixtures/codex/simple_session.jsonl`
- `fixtures/codex/duplicate_snapshot.jsonl`
- `fixtures/codex/subagent_inherited_history.jsonl`
- `fixtures/codex/malformed_lines.jsonl`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- Added a source-isolated `CodexSessionAdapter` with read-only detection,
  recursive JSONL discovery, streaming line parsing, and a shared async adapter
  contract.
- Normalized Codex input/cache/output/reasoning usage without persisting prompt,
  completion, or source-code bodies; unknown fields remain unavailable.
- Added stable event hashes, file offsets, file-head signatures, cumulative
  snapshot deltas, reset generations, malformed-line skipping, and inherited
  history suppression.
- Added session metadata and parent/child session aggregation, plus testable
  macOS/Windows default path resolution branches and custom path detection.

## Verification performed

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets` — 12 Rust tests passed
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Remaining limitations

- The importer is not connected to the desktop startup flow yet; Phase 4 adds
  the Tauri commands and screens that consume persisted data.
- File watching is still pending, and Claude/OTel/CC Switch/Cockpit adapters
  are intentionally out of scope for T001–T014.
- The optional local proxy remains explicitly unimplemented.
