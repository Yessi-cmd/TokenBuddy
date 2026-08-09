# Windows single-manifest linking

## Purpose

Ensure every Windows desktop target receives Common Controls v6 while preventing duplicate Manifest resources in production and test binaries.

## Affected files

- `apps/desktop/src-tauri/build.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `docs/change-logs/2026-08-09-02-windows-single-manifest.md`

## Behaviour changes

- Tauri build-time generation still provides Windows icon and version resources, but no longer embeds its default application Manifest.
- The project-owned `windows-app-manifest.xml` is linked once into every supported Windows target, including production binaries and library test binaries.
- The empty desktop bin test harness remains enabled; no test target is skipped to make CI pass.
- The rejected test-only linker directive from the previous batch is removed because Cargo does not classify library unit tests as standalone test targets.

## Verification performed

- Confirmed against `tauri-build` 2.6.3 API documentation and source that `WindowsAttributes::new_without_app_manifest()` disables only the default application Manifest while retaining the remaining Windows resource generation.
- `git diff --check` passed.
- `pnpm --filter @tokenbuddy/desktop format:check` passed.
- `pnpm --filter @tokenbuddy/desktop lint` passed.
- `pnpm --filter @tokenbuddy/desktop test` passed: 2 test files, 55 tests.
- `pnpm build:web` passed, including TypeScript project compilation and the Vite production build.
- GitHub Actions run `31285816422` supplied the exact `rustfmt` diff for `build.rs`; the file was updated to match it before the next compiler run.

## Remaining limitations

- The Windows linker and runtime result requires confirmation from GitHub Actions because the local machine does not have Rust/MSVC installed.
- Windows tray interaction, installer UX, update-dialog UX, high-DPI behaviour, hidden-window collection, and CPU/P95 measurements still require Windows real-machine validation.
