# Claude streamed usage reconciliation and OpenCode Go pricing

## Purpose

Fix Claude Code responses that were displayed as `Unavailable` input/cache and zero output when a provisional streamed usage row arrived before the complete row, and add an official OpenCode Go price card for the frequently used `deepseek-v4-flash` model.

## Affected files

- `fixtures/claude/streamed_usage_enrichment.jsonl` — new sanitized provisional/final streamed-response schema fixture.
- `crates/adapters/claude-session/src/lib.rs` — parser identity regression coverage for the new fixture.
- `crates/core/tests/phase3_claude.rs` — end-to-end Core regression coverage that requires one complete event rather than one provisional event.
- `crates/storage/src/lib.rs` — same-hash usage enrichment, provider-aware price lookup, and estimate refresh after provider attribution.
- `crates/storage/src/pricing.rs` — OpenCode Go `deepseek-v4-flash` price card, strictly bound to the official Go endpoint.
- `crates/storage/src/migrations.rs` and `crates/storage/migrations/0010_claude_streamed_usage_reimport.sql` — one-time safe Claude cursor reset so existing rows are reconciled from their original read-only transcripts.
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md` — implementation status update.

## Behaviour changes

- When Claude Code writes a provisional `{input_tokens: 0, output_tokens: 0}` usage object and later writes complete usage with the same message ID, TokenBuddy keeps one stable event and enriches it with the complete input, cache-read, cache-write, and output values.
- Existing installations clear only the Claude Session import cursors during migration 0010. The next normal scan rereads Claude JSONL read-only and reconciles the existing stable hashes; it does not delete usage events or increase event counts.
- OpenCode Go `deepseek-v4-flash` estimates use the official per-million-token rates: USD 0.14 uncached input, USD 0.0028 cached read, and USD 0.28 output. No cache-write charge is invented where the official table shows none.
- Pricing source: `https://dev.opencode.ai/docs/go/` (checked 2026-08-09).
- The rate card matches the official `https://opencode.ai/zen/go` route, not a mutable display name or the model name alone. Other relays exposing `deepseek-v4-flash` remain `N/A` unless their own authoritative pricing is added.
- Cost estimates are recalculated after provider attribution. A provider change can add a newly valid estimate or clear an estimate that no longer belongs to the resolved route; provider-reported cost remains authoritative.

## Verification performed

- `pnpm --filter @tokenbuddy/desktop format:check` — passed.
- `pnpm --filter @tokenbuddy/desktop lint` — passed.
- `pnpm --filter @tokenbuddy/desktop test` — passed, 55 tests.
- `pnpm --filter @tokenbuddy/desktop build` — passed.
- Applied migrations 0001 through 0010 in order to an in-memory SQLite database with Python's standard `sqlite3`; every migration executed successfully.
- `git diff --check` — passed.
- `cargo +1.97.1-x86_64-pc-windows-gnu clippy -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets --all-features -- -D warnings` — passed.
- `cargo +1.97.1-x86_64-pc-windows-gnu test -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets` — passed, 67 tests across the selected packages and integration targets.
- `cargo +1.97.1-x86_64-pc-windows-gnu check -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets` — passed.
- Added Rust unit/integration coverage for provisional-to-final usage reconciliation, repeated-import idempotency, Core aggregate correctness, official-route pricing, and rejection of an unrelated relay.

## Remaining limitations

- Full Windows desktop linking remains pending in CI: this machine has neither MSVC Build Tools (`link.exe`) nor a GNU linker configuration capable of linking the Tauri desktop DLL. The affected adapter/storage/core packages compile, lint, and test successfully with the pinned Rust 1.97.1 GNU toolchain.
- OpenCode Go is a subscription with dollar-denominated usage limits. TokenBuddy displays the token-based calculation as an API-equivalent estimate, not as a claim about the user's final invoice or remaining subscription quota.
- Future OpenCode Go price changes require updating the static price card and its source date.
