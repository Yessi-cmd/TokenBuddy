# TokenBuddy Agent Instructions

## Specification and change log requirements

- Read `AI_Coding_Token_Observatory_PROJECT_SPEC.md` in full before modifying code. Treat it as the canonical product, architecture, implementation, and acceptance specification.
- Create a Markdown change log in `docs/change-logs/` for every coherent code-change batch.
- Name entries `YYYY-MM-DD-NN-brief-topic.md`.
- Each entry must state the purpose, affected files, behaviour changes, verification performed, and any remaining limitations.
- Update the implementation status at the end of the project specification whenever a milestone is completed.
- Do not overwrite or remove existing user changes. Record only changes made in the current batch.

## Project rules

- Use `main` as the default and only persistent branch. Create temporary branches only when isolation is genuinely needed, then merge and remove them promptly.
- Keep the prescribed stack: Tauri 2, React, TypeScript, Vite, Rust, SQLite, and shared core code for macOS and Windows.
- Keep external data sources behind independent adapters. Schema changes or failures in one adapter must not prevent the application or other adapters from working.
- Start parser work with sanitized fixtures. Do not use or modify real Codex, Claude Code, CC Switch, or Cockpit data as development fixtures.
- Treat CC Switch and Cockpit integrations as read-only. Never modify their databases, settings, credentials, or runtime configuration.
- Keep the local proxy optional and outside the MVP. It must never become a prerequisite for application startup or token statistics.
- The frontend must access persisted data through Tauri commands or application services, never by opening SQLite directly.

## Data correctness and privacy rules

- Preserve missing values as `Unavailable` or `None`; never turn unknown token counts, costs, quotas, or attribution into zero.
- Prefer provider-reported usage over OTel, session logs, proxy logs, and tokenizer estimates, in that order. Surface the applicable precision level in the UI.
- Preserve raw usage semantics, generate stable event hashes, import incrementally, and keep repeated imports idempotent.
- Handle cumulative snapshots, file truncation and rotation, malformed records, and inherited sub-agent history without double counting or writing negative deltas.
- Keep official quota windows separate from raw token usage. Never infer exact subscription tokens from quota percentages.
- Do not save prompt text, completion text, source code, authorization headers, cookies, full API keys, OAuth tokens, or refresh tokens by default.
- Redact secrets from logs. Store credentials only in the OS keychain when credential management is explicitly in scope.
- Do not apply official model pricing to a third-party provider and present it as actual cost. Missing provider pricing must remain unavailable.

## Verification rules

- Add or update tests before implementation where practical, including fixtures for every supported log schema.
- Never rewrite an existing fixture merely to make a regression test pass; add a new fixture for a new schema variant.
- Run the relevant formatter, linter, unit tests, integration tests, and build checks for every change batch, and record the exact verification in its change log.
- Preserve cross-platform path handling and avoid macOS-only assumptions. Keep Windows compilation covered by CI.
- A repeated import of the same fixture must not increase event counts or change session aggregates.
