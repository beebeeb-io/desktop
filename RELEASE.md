# Releasing the Beebeeb desktop app

The step-by-step release procedure is [`docs/RELEASING.md`](docs/RELEASING.md). This file is the overview of the
native release path (0.9.0 and later). The 0.8.x Tauri builds (macOS `.dmg`, Windows `.msi`, Linux `.AppImage` /
`.deb` / `.rpm`) and their steps are gone in 0.9.0 on every OS; their history is this file at tag `desktop-v0.8.25`.

## The native release workflow

[`.github/workflows/release-native-windows.yml`](.github/workflows/release-native-windows.yml) releases the native
app (`native/beebeeb-drive-rune` on the engine `crates/beebeeb-engine`), Windows only, started by hand
(`workflow_dispatch`). In order, it:

1. Validates the version and channel: plain semver, 0.9.0 or later. A 0.8.x version is refused.
2. Refuses while `RELEASE_BLOCKERS.md` lists an item that binds Windows.
3. Checks the authored release notes (job `validate-release-notes`): `RELEASE_NOTES.md` must name the version and
   carry the "macOS: not in this release" line (`scripts/release-channel-checks.mjs … --no-macos`). Authored notes
   are mandatory; an auto-generated changelog is never enough.
4. Verifies the release branch point (the reviewed head of `main` or a `release/*` branch).
5. Builds the installer with [`packaging/windows/build-installer.ps1`](packaging/windows/build-installer.ps1)
   (plain `makensis`) and runs the Authenticode gate on the artefacts (a self-test, then the artefacts).
6. Publishes the v2 channel manifest (job `publish-manifest`): it writes `desktop/v2/<channel>.json`, signs it with
   the native line's updater key, and verifies the signature against the committed public key
   `crates/beebeeb-updater/keys/updater-v2.pub` before anything is published (step "Verify the manifest's
   signature against the committed v2 key").

The manifests that 0.8.24 and older read (`desktop/latest.json`, `beta.json`, `alpha.json`) are never written by
this workflow, so a native build reaches only clients that passed through the 0.8.25 bridge. Whether and when the
0.8.25 bridge is promoted to beta and stable is the maintainers' decision, made on the 0.8.x line.

## Versioning

The release version is the workflow's `version` input; the release is tagged `desktop-v<version>`. Releases of
0.9.0 and later come only from this workflow.

## Signing

- **Windows code signing (Authenticode):** `docs/RELEASING.md` → "Windows Authenticode signing (Azure Artifact
  Signing)".
- **Update signing:** the two public keys live in `crates/beebeeb-updater/keys/`: `updater-0.8.pub` (the 0.8.x line)
  and `updater-v2.pub` (the native line). The private key of the native line exists only as secrets of the GitHub
  environment `desktop-release` (required reviewer, `main` and `release/*` only); the jobs that sign name that
  environment.
- **Rotating an update key** is a two-release process, because an installed app trusts only the keys it shipped
  with: release `N` is signed with the old key and carries the new public key; release `N+1` and later are signed
  with the new key. Never publish an update signed by a key the installed app does not already trust: the updater
  rejects it, and every user would need a manual reinstall. The procedure and the key guard:
  `docs/RELEASING.md` → "Updater Signing Key Rotation" and "Signature-key guard".

## macOS and Linux

- **macOS:** no 0.9.0 build yet. The native macOS app's packaging, signing, notarisation and update steps are
  carried in
  [`native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md`](native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md).
  The 0.8.x copy of this file (tag `desktop-v0.8.25`) keeps the Tauri-era sections the macOS phase may read: the
  production signing contract, login item and daemon start-up, File Provider domain persistence, and the safe
  uninstall.
- **Linux:** no 0.9.0 build.
