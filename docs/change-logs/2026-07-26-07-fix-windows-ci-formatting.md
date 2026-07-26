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
- GitHub Actions run [30181703574](https://github.com/Yessi-cmd/TokenBuddy/actions/runs/30181703574) passed on both `macos-15` and `windows-latest`.

## Remaining limitations

- GitHub Actions reports a non-blocking warning that several actions still target Node.js 20 while the runner forces Node.js 24; this is unrelated to the formatting failure.
