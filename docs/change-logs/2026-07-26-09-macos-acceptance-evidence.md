# macOS Phase 4b acceptance evidence

## Purpose

Make the hidden-window lifecycle observable in a debug-only build and complete a real macOS smoke test for the shared SPA, loopback settings, continued collection after close, and normal accessory startup.

## Affected files

- `apps/desktop/src-tauri/src/lib.rs`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-07-26-09-macos-acceptance-evidence.md`

## Behaviour changes

- Debug builds accept the task-specific `TOKENBUDDY_DEBUG_SHOW_WINDOWS=1` switch to show the main window for UI acceptance. The switch is opt-in and has no effect on release builds.
- The default launch path remains tray/accessory-first: both windows start hidden and macOS uses the accessory activation policy.
- The debug-visible path makes the existing close handler observable without changing its production behavior: closing the full window hides it while the process and Core continue running.

## Verification

- `cargo test -p tokenbuddy-desktop --all-targets` passed: 9 desktop/web tests.
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle` passed.
- `pnpm --filter @tokenbuddy/desktop tauri build --debug` passed and produced the macOS app and DMG bundles.
- Computer Use on the real macOS build opened `/providers`, `/quotas`, and `/settings`; the three pages rendered their expected headings and explicit `Unavailable` states.
- The real debug app started the loopback API, changed its persisted Codex Home through `PUT /api/settings` to a sanitized fixture, and restored `/Users/zhongyan/.codex` afterward.
- After closing the visible dashboard, appending `phase4b-hidden-request` to the sanitized JSONL produced `total=3` and `input_tokens_total=20` through `/api/usage-events`, proving collection continued while the window was hidden.
- Normal accessory startup after clearing the debug switch kept the packaged process alive. A steady 40-sample CPU run at 250 ms intervals measured average `0.00%`, P95 `0.00%`, and maximum `0.00%` after the one-time path-switch import completed.
- `cargo check --target x86_64-pc-windows-gnu --workspace --all-targets` was attempted; it stopped at the local toolchain boundary because `x86_64-w64-mingw32-gcc` is not installed. The repository's native Windows CI matrix remains the authoritative cross-platform build path.

## Remaining limitations

- This host has no Windows desktop session, so Windows tray click/double-click/right-click behaviour, hidden-window collection, and Windows CPU/P95 remain unverified.
- The local macOS host also lacks the MinGW linker needed for a useful Windows cross-check; the failed check is environmental, not a source-level Windows test result.
- The Computer Use bridge cannot expose the macOS `SystemUIServer` status-item tree, so the TokenBuddy status-bar icon itself could not be clicked directly; the app's real accessory startup and close/hide/Core lifecycle were verified through the packaged process and visible-window path.
