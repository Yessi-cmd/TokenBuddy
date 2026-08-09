# Release updater manifest fix

## Purpose

Restore Windows release artifact uploads after Tauri 2 changed its updater artifact layout.

## Affected files

- `.github/workflows/release.yml`
- `docs/change-logs/2026-08-09-03-release-updater-manifest.md`

## Behaviour changes

- The release workflow now uses the Tauri v2 NSIS installer (`*-setup.exe`) as the Windows updater payload instead of looking for the legacy `*.nsis.zip` artifact.
- `latest.json` now embeds the generated `.sig` file content directly, as required by the Tauri updater contract.
- Manifest generation uses Node.js, which the workflow installs explicitly, instead of relying on a `python3` executable being available on the Windows runner.

## Verification

- Confirmed that release run `31286660505` completed the Windows installer build and failed only at the previous updater-manifest step.
- Checked the current Tauri v2 updater documentation: `createUpdaterArtifacts: true` generates `*-setup.exe` and `*-setup.exe.sig` on Windows, and the static manifest must contain the `.sig` file content.
- A new manual Release workflow run will provide the end-to-end verification for manifest generation and artifact upload.

## Remaining limitations

- The installers are not Authenticode-signed, so Windows SmartScreen may warn on interactive installation.
- The current application version remains `0.1.3`; this manual workflow run produces test artifacts and does not publish or replace the existing GitHub release.
