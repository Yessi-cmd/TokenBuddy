# Complete Phase 0 workspace setup

## Purpose

Complete the repository-initialization milestone required before the shared
domain and storage work, while keeping the existing Tauri desktop shell intact.

## Affected files

- `Cargo.toml`
- `crates/domain/Cargo.toml`
- `crates/domain/src/lib.rs`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- The Rust workspace now includes a platform-neutral `tokenbuddy-domain`
  crate, ready to hold shared types for the desktop app and adapters.
- The Tauri 2, React, TypeScript, Vite, Rust verification commands and the
  existing macOS/Windows CI configuration remain the project entry points.

## Verification performed

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `pnpm --filter @tokenbuddy/desktop format:check`
- `pnpm --filter @tokenbuddy/desktop lint`
- `pnpm --filter @tokenbuddy/desktop test`
- `pnpm --filter @tokenbuddy/desktop build`

## Remaining limitations

- Windows CI can only be confirmed after the workflow runs on GitHub Actions.
- The domain crate is a scaffold; Phase 1 supplies the actual shared model and
  persistence layer.
