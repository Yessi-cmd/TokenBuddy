# Fix Windows CI formatting check

## Purpose

Prevent the Windows GitHub Actions runner from converting repository text files to CRLF and causing the Prettier check to fail while the macOS job passes.

## Affected files

- `.gitattributes`
- `docs/change-logs/2026-07-26-07-fix-windows-ci-formatting.md`

## Behaviour changes

- Git now keeps recognized text files at LF on checkout across platforms.
- Binary files remain governed by Git's automatic text detection.

## Verification

- `pnpm format:check` passed.
- `pnpm lint` passed.
- `pnpm test` passed: 2 frontend tests and 17 Rust tests.
- `pnpm build:web` passed.
- `pnpm check:rust` passed.
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle` passed.
- `git diff --check` passed.
- Pending: rerun GitHub Actions on a commit containing this change.

## Remaining limitations

- The Windows runner must be rerun to confirm the original CI failure is resolved in the hosted environment.
