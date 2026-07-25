# Complete Phase 4 desktop panel and CI coverage

## Purpose

Connect the persisted data layer to the Tauri desktop application and deliver
the first usable dashboard, session list/detail view, Codex path controls, and
cross-platform CI compile/test coverage.

## Affected files

- `.github/workflows/ci.yml`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/App.tsx`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/styles.css`
- `crates/storage/src/lib.rs`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- The Tauri app now opens a per-user SQLite database and exposes dashboard,
  session list/detail, usage-event, source, Codex-path detection, and explicit
  read-only Codex rescan commands.
- The frontend now renders daily token totals, cache hit rate, cost
  availability, session summaries, request timelines, precision badges, and
  unavailable-value states without opening SQLite directly.
- Added a custom Codex Home field while retaining platform-specific default
  path detection; a scan is explicit and never modifies Codex files.
- CI now verifies frontend and Rust tests/lint/checks and compiles the Tauri
  application on both the macOS and Windows matrix without bundling.

## Verification performed

- `pnpm format:check`
- `pnpm lint`
- `pnpm test` — frontend test plus 12 Rust tests passed
- `pnpm build:web`
- `pnpm check:rust`
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`

## Remaining limitations

- GitHub Actions has not yet run in this repository, so Windows CI remains a
  configured but remotely unverified path.
- Claude Session, OTel, CC Switch, Cockpit, file watching, export, and quota
  features remain outside T001–T014.
- The optional local proxy is intentionally not implemented.
