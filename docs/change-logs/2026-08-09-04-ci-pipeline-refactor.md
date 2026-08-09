# CI pipeline refactor: stop push-to-main compile failures and notification spam

## Purpose

The GitHub Actions `CI` workflow had been failing on most pushes to `main` (7 of the last 10 runs), each failure sending a mobile notification. The failures were mostly pipeline-design defects rather than code defects, so they kept recurring. This batch refactors the pipeline so those failure classes cannot recur, cancels superseded runs so a burst of pushes only notifies once, and stops the slow Tauri build from running on unrelated commits.

## Affected files

- `rust-toolchain.toml` (new) — pins the Rust toolchain to `1.97.1`, the stable version last verified green in CI.
- `.github/workflows/ci.yml` — restructured into three jobs with concurrency cancellation, per-job timeouts, toolchain-aware Cargo caching, and path-gated Tauri build.
- `.github/workflows/release.yml` — toolchain install step now reads `rust-toolchain.toml`, so release builds use the same pinned Rust as CI and local dev.
- `docs/change-logs/2026-08-09-04-ci-pipeline-refactor.md` (this file).

## Behaviour changes

- **Pinned toolchain.** `rust-toolchain.toml` fixes the version (`1.97.1`), profile (`minimal`), and components (`rustfmt`, `clippy`). The CI and release workflows install from that file instead of `stable`, so `cargo fmt --check` and `cargo clippy -D warnings` can no longer drift between the developer machine and CI when Rust releases a new stable.
- **Concurrency cancellation.** `ci` runs for the same branch cancel any in-progress run (`concurrency.group: ci-${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress: true`). A burst of pushes now runs and notifies only for the final commit.
- **Three focused jobs instead of one matrix job:**
  - `frontend` (Ubuntu, once): Prettier `format:check`, ESLint `lint`, Vitest `test`, and `tsc -b && vite build`. Platform-independent checks now report in ~1 minute.
  - `rust` (macOS + Windows): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-targets`, `cargo check --workspace --all-targets`. Windows compile coverage (required by AGENTS.md) is preserved.
  - `package` (macOS + Windows, `needs: [frontend, rust]`): `tauri build --debug --no-bundle`, gated by `dorny/paths-filter@v3` to `crates/**`, `apps/desktop/**`, `Cargo.toml`, `Cargo.lock`, `pnpm-lock.yaml`, `package.json`, `rust-toolchain.toml`, `rustfmt.toml`, and the workflow itself. Docs-only commits skip it; if a dependency job fails, the whole job is skipped rather than failing a second time.
- **Toolchain-aware Cargo cache.** `Swatinem/rust-cache@v2` replaces the hand-rolled `actions/cache` that keyed only on `Cargo.lock` and could restore a stale `target/` from a different toolchain (a former source of flaky `linking with link.exe failed: exit code 1123` on Windows).
- **Per-job timeouts** (15/40/45 minutes) so a hung build cannot consume the full runner budget.
- The root `package.json` scripts are unchanged; the job split only changes how CI invokes the same underlying tools. `release.yml` build/release logic is otherwise untouched.

## Verification performed

- `git diff --check` passed.
- Both workflow files parse as valid YAML.
- `gh run list` confirmed the previous failure pattern (7 of 10 runs failing) and that the current HEAD run `31287610490` was green on both platforms before this batch.
- CI verification of this batch is pending: a push/PR run must show `frontend` green once, `rust` green on both macOS and Windows, and `package` green on both (or skipped when only `docs/**` changed).

## Remaining limitations

- Rust is pinned to `1.97.1`; moving to a newer stable requires bumping `rust-toolchain.toml` and re-verifying (intentional trade for reproducibility).
- `package` depends on `frontend` + `rust` passing; a Windows-only Rust test failure skips the packaging build on both OSes for that run (fewer failure notifications, at the cost of not separately exercising the packaging step on the failing run).
- Notifications for failed runs are GitHub's default behaviour; the refactor reduces how often runs fail and how many per burst, but a genuinely broken commit still notifies.
- Local verification was limited: this machine has no Rust/pnpm toolchain, so the full five-command suite could not be run here.
