# Windows updater build fix

## Purpose

Restore the Windows CI build after the updater check moved its `AppHandle` into an async block and then attempted to reuse it for the native update prompt.

## Affected files

- `apps/desktop/src-tauri/src/lib.rs`
- `docs/change-logs/2026-08-08-01-windows-updater-build.md`

## Behaviour changes

- The Windows update checker now gives the asynchronous update request its own cloned `AppHandle`.
- The original handle remains available for dispatching the update confirmation dialog on the main thread.
- Update timing, error handling, download, installation, and restart behaviour are unchanged.

## Verification performed

- `git diff --check` passed.
- `pnpm --filter @tokenbuddy/desktop format:check` passed.
- `pnpm --filter @tokenbuddy/desktop lint` passed.
- `pnpm --filter @tokenbuddy/desktop test` passed: 2 test files, 55 tests.
- `pnpm build:web` passed, including TypeScript project compilation and the Vite production build.
- Rust formatting, Clippy, workspace tests, and the Windows Tauri build were not run locally because this Windows machine does not have Cargo/Rust or MSVC installed; the next GitHub Actions Windows job must provide final compiler confirmation.

## Remaining limitations

- Windows tray interaction, installer UX, update-dialog UX, high-DPI behaviour, hidden-window collection, and CPU/P95 measurements still require Windows real-machine validation.
