# Fix cross-source double counting; attribute the real provider

## Purpose

The CC-Switch adapter shipped in the previous batch imported proxied requests as
usage events. Those are the *same* API calls Codex/Claude Code already record in
their own transcripts, so every proxied call was counted twice. This turns
CC-Switch and Cockpit into provider **attribution** sources instead, and adds the
model × provider breakdown the counts now support.

## The defect, with evidence

On the author's real machine:

- `proxy_request_logs` held **1,731** rows with `data_source='proxy'` —
  **17,088,579 input + 1,244,489 output tokens, $112.79** — all imported as
  usage events.
- Those rows span **15 distinct `session_id`s, and 15/15** resolve to an existing
  `~/.claude/projects/**/<session_id>.jsonl` transcript that the Claude adapter
  imports independently.
- Sampled session `9f40a9e0-…`: the Claude transcript holds 255 assistant usage
  rows for `deepseek-v4-pro` ending at `15:03:37.501Z`; the CC-Switch proxy row
  for the same session and model is stamped `15:03:37`. Same call, two records.
- The identifiers never matched (CC-Switch mints `session:<uuid>`, Claude uses
  `requestId`/`message.id`), so the request-identity dedup could not catch it.

## Fix

Spec §6.1 ranks session logs above proxy logs, and §10.1/§11.3 say CC-Switch and
Cockpit supply provider/account context rather than being the token source. The
adapters now follow that:

- **CC-Switch** emits no usage events. It reads `proxy_request_logs` only to
  learn *which provider served which session*, minting the session id exactly as
  the native adapter does (`claude-code-session:<short_hash>` /
  `codex-session:<short_hash>`) so the attribution lands on that adapter's rows.
- **Cockpit** emits no usage events either — Codex's rollout log already counted
  those requests. `request_logs` carries no session id, so per-session
  attribution is impossible; it contributes provider context only (spec §11.3).
- New `SessionProviderAttribution` domain type + `session_provider_attributions`
  table (migration `0004`). Applying an attribution backfills already-stored
  events and is consulted when inserting new ones, so the result is independent
  of import order.
- Provider precedence on insert: launcher-reported truth > identity the adapter
  resolved > provider guessed from the model name.

This also fixes attribution that the model name cannot express: `deepseek-v4-pro`
reached through a Claude-compatible relay was labelled **Anthropic** (no prefix
match → fell back to the app kind); it is now **DeepSeek**, as reported by
CC-Switch.

## Model × provider breakdown

`model_breakdown` groups usage by model, serving provider, and app under the same
`UsageFilters` as the dashboard, so the table always adds up to the headline
numbers. Wired through core → Tauri command `get_model_breakdown` →
`/api/model-breakdown` → `api.ts` → a new dashboard section (horizontally
scrollable, tabular numerals, `Unavailable` preserved).

## Verification

- `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`
- `cargo test --workspace --all-targets` — 11 suites pass, including new
  regressions: CC-Switch/Cockpit emit no usage events, CC-Switch mints native
  session ids, and a storage test proving a guessed provider is corrected by an
  attribution and never reappears for later events.
- Real data: CC-Switch import now yields **0 usage events (was 1,731)** and 15
  attributions across 2 real providers. The attributed ids were independently
  recomputed in Python and matched, and each session's Claude transcript exists.
- `cargo build -p tokenbuddy-desktop`; `prettier --check`, `eslint
  --max-warnings 0`, `vitest run` (9), `vite build`.

## Note on existing databases

The double-counted rows already written by the previous build are removed the
next time their source file is re-read only if the DB is reset; users who ran
that build may want to delete `tokenbuddy.sqlite3` and rescan for clean totals.
