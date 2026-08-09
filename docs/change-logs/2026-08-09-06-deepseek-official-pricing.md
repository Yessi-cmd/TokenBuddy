# DeepSeek official API pricing

## Purpose

Add an authoritative `deepseek-v4-flash` price card for requests routed directly to DeepSeek's official API, while preserving `N/A` for unrelated relays that expose the same model name.

## Affected files

- `crates/storage/src/pricing.rs` — official DeepSeek endpoint recognition, V4 Flash price rule, and unit tests.
- `crates/storage/src/lib.rs` — provider-attribution integration test covering the Anthropic-compatible official endpoint.
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md` — implementation status update.

## Behaviour changes

- `deepseek-v4-flash` requests attributed to `https://api.deepseek.com` or its official subpaths, including `/anthropic` and `/v1`, receive an API-equivalent USD estimate.
- The official per-million-token rates are USD 0.14 for cache-miss/uncached input, USD 0.0028 for cache-hit input, and USD 0.28 for output. No separate cache-write fee is added.
- A `deepseek` provider ID inferred only from the model name is insufficient for pricing. A provider record with the official endpoint is required, preventing third-party relays from inheriting DeepSeek's official price.
- Pricing source: `https://api-docs.deepseek.com/quick_start/pricing` (checked 2026-08-09).

## Verification performed

- `pnpm --filter @tokenbuddy/desktop format:check` — passed.
- `pnpm --filter @tokenbuddy/desktop lint` — passed.
- `pnpm --filter @tokenbuddy/desktop test` — passed, 55 tests.
- `pnpm --filter @tokenbuddy/desktop build` — passed.
- `git diff --check` — passed.
- Added Rust unit coverage for the official base, Anthropic-compatible and `/v1` endpoints, plus rejection of a model-derived provider without an endpoint.
- `cargo +1.97.1-x86_64-pc-windows-gnu clippy -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets --all-features -- -D warnings` — passed.
- `cargo +1.97.1-x86_64-pc-windows-gnu test -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets` — passed, 67 tests across the selected packages and integration targets.
- `cargo +1.97.1-x86_64-pc-windows-gnu check -p tokenbuddy-claude-session -p tokenbuddy-storage -p tokenbuddy-core --all-targets` — passed.
- Added Storage integration coverage proving an official DeepSeek attribution receives the expected estimate.

## Remaining limitations

- Full Windows desktop linking remains pending in CI: this machine has neither MSVC Build Tools (`link.exe`) nor a GNU linker configuration capable of linking the Tauri desktop DLL. The affected adapter/storage/core packages compile, lint, and test successfully with the pinned Rust 1.97.1 GNU toolchain.
- Static prices may change; the official DeepSeek pricing page must be checked when updating the application.
- TokenBuddy reports this as an estimate unless the provider itself supplies a cost value; provider-reported cost remains authoritative.
