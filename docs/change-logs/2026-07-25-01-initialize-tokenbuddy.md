# Initialize the TokenBuddy repository

## Purpose

Establish the initial Git repository and a reproducible Tauri 2, React,
TypeScript, Vite, and Rust development environment for TokenBuddy.

## Affected files

- Renamed the product within `AI_Coding_Token_Observatory_PROJECT_SPEC.md`.
- Added root Git, pnpm workspace, Cargo workspace, formatting, and ignore
  configuration.
- Added the React and TypeScript desktop frontend under `apps/desktop/`.
- Added the Tauri Rust application, IPC example, capabilities, and desktop
  icons under `apps/desktop/src-tauri/`.
- Added macOS and Windows verification in `.github/workflows/ci.yml`.
- Added project setup and verification commands to `README.md`.
- Added JavaScript and Rust lockfiles.

## Behaviour changes

- The product is now named TokenBuddy throughout the project specification.
- `pnpm dev` starts the Tauri development application.
- The initial screen includes a small IPC connectivity check backed by the
  Rust `greet` command.
- React Query is initialized at the application root. Zustand and ECharts are
  installed for the state and charting work described by the specification.
- The repository provides shared commands for formatting, linting, testing,
  frontend builds, Rust checks, and Tauri builds.

## Verification performed

- `pnpm --filter @tokenbuddy/desktop format:check`
- `pnpm --filter @tokenbuddy/desktop lint`
- `pnpm --filter @tokenbuddy/desktop test` — 1 test passed
- `pnpm --filter @tokenbuddy/desktop build`
- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets` — 1 test passed
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`
- Launched `target/debug/tokenbuddy-desktop` on macOS and confirmed it
  remained running until the smoke test terminated it.

## Remaining limitations

- The macOS and Windows CI workflow has been created but cannot run until the
  repository is hosted and pushed to GitHub.
- The generated app icon is an initialization placeholder, not final branding.
- The application is only the Phase 0 shell. Database migrations, domain
  types, adapters, fixtures, and observability screens are not implemented.
- Phase 0 remains unchecked in the project specification until remote Windows
  CI is verified.
