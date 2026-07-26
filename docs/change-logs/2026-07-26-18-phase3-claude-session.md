# Phase 3 Claude Session

## Purpose

Implement the Phase 3 Claude Code Session MVP according to the project specification: read-only JSONL ingestion, two versioned schema paths plus a conservative fallback, persistent incremental cursors, session/sub-agent metadata, and shared Core/query integration.

## Affected files

- `crates/adapters/claude-session/` — new Claude Session adapter and tests.
- `fixtures/claude/` — new sanitized V1, V2, sub-agent, and malformed JSONL fixtures.
- `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/core/tests/phase3_claude.rs` — Claude Home configuration, independent multi-adapter refresh, watcher targets, error isolation, and Core integration tests.
- `Cargo.toml`, `Cargo.lock`, `apps/desktop/src-tauri/Cargo.toml` — workspace and desktop dependency wiring.
- `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/web.rs`, `apps/desktop/src/lib/api.ts` — Claude detection/rescan commands and loopback API routes.
- `apps/desktop/src/App.tsx`, `apps/desktop/src/App.test.tsx`, `apps/desktop/src/styles.css` — Claude source controls, scan status, and settings copy.
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md` — Phase 3 implementation status and current limitations.

## Behaviour changes

- Claude Code defaults to `~/.claude` on macOS/Linux and `%USERPROFILE%\.claude` on Windows; `Claude Home` remains user-configurable and persisted through Core settings.
- `ClaudeSessionAdapter` recursively imports read-only `projects/**/*.jsonl` files and reports source health independently from Codex.
- Versioned extraction accepts canonical message usage (V1), top-level/session-event usage variants (V2), and a fallback that only accepts explicit stable usage fields.
- Anthropic usage maps `input_tokens`, cache creation, cache read, and output fields into the shared normalized model. `input_tokens_total` remains unavailable when its component fields are missing; no unknown value is converted to zero.
- Incremental cursors handle repeated imports, file truncation/rotation, incomplete final lines, malformed records, and cumulative snapshots. Inherited history is skipped while explicit child sessions retain parent-session linkage.
- Only the usage object is retained as raw data. Prompt/completion bodies, source text, and `costUSD` are not persisted; provider and cost attribution remain unavailable unless a later provider/OTel adapter supplies them.
- Core continues importing the other source when one adapter fails and records the failed source health state. Tauri IPC, loopback HTTP, and the shared React SPA expose Claude detection and rescan actions.

## Verification

- `pnpm format:check`
- `pnpm lint`
- `pnpm test`
- `pnpm build:web`
- `pnpm check:rust`
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`
- `git diff --check`

All commands passed. The Claude adapter tests cover both schema variants, privacy redaction boundary, malformed records, sub-agent/inherited history, partial-line retry, rotation, and idempotent cursors. Core tests cover shared queries, watcher-driven append import, and isolation from an invalid Claude source.

## Remaining limitations

- Claude OTel, CC Switch, Cockpit, official quota, and proxy adapters remain out of scope for this batch.
- Real user Claude Code directories were not read or used as fixtures; Windows GUI/installer and real cross-platform runtime acceptance still require their respective environments.
- Unknown future Claude fields are intentionally left as `Unavailable` until a new sanitized fixture and parser variant are added.
