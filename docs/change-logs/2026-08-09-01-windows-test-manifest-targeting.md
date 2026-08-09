# Windows test manifest targeting

## Purpose

Allow the Windows CI test suite to link while preserving the Common Controls v6 manifest required by the Tauri MockRuntime tests.

## Affected files

- `apps/desktop/src-tauri/build.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `docs/change-logs/2026-08-09-01-windows-test-manifest-targeting.md`

## Behaviour changes

- The custom Common Controls v6 manifest linker arguments now apply only to Rust test targets.
- The desktop entry-point binary no longer builds an empty test harness; all desktop tests remain in the library target.
- Production Windows binaries continue to receive their normal Tauri-generated resources and manifest.
- Windows library tests continue to receive the manifest needed to load `TaskDialogIndirect` from Common Controls v6.

## Verification performed

- Confirmed against the official Cargo build-script reference that `rustc-link-arg-tests` targets only Rust test targets.
- `git diff --check` passed.
- `pnpm --filter @tokenbuddy/desktop format:check` passed.
- `pnpm --filter @tokenbuddy/desktop lint` passed.
- `pnpm --filter @tokenbuddy/desktop test` passed: 2 test files, 55 tests.
- `pnpm build:web` passed, including TypeScript project compilation and the Vite production build.

## Remaining limitations

- The Windows-specific linker result requires confirmation from GitHub Actions because the local machine does not have Rust/MSVC installed.
- Windows tray interaction, installer UX, update-dialog UX, high-DPI behaviour, hidden-window collection, and CPU/P95 measurements still require Windows real-machine validation.
