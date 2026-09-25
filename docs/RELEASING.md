# Desktop Releasing

Desktop releases ship with authored, workbench-style release notes. The generated GitHub changelog still matters, but it is appended after the authored notes instead of being the only release body.

## Release Notes Template

Create `RELEASE_NOTES.md` at the repository root for the exact plain semver version being released. Do not keep a reusable placeholder in the repo root; stale notes should make the release workflow fail.

Use this section shape:

```markdown
# Beebeeb Desktop X.Y.Z - <short tagline>

Write a 2-4 sentence intro in plain, honest language. Explain what changed and why it matters without hype, emojis, or vague reassurance. If there is a known limitation, say so here instead of burying it.

### What's New

- Describe the user-visible feature or workflow change.
- Mention changed defaults, supported platforms, or install behavior when relevant.

### Bug Fixes / Hardening

- **<Fix headline>:** State what was broken, who it affected, and what changed. Verified fixed by <exact command, test, device run, or release dry-run>.
- **<Hardening headline>:** State the failure mode being prevented and how the new behavior was verified.

### Verification

- Real hardware smoke test: <device, OS version, installer/update path, and the exact workflow tested>.
- Test suite: `<command>` completed with <passed>/<total> passing.
- Release workflow check: <what was checked, for example actionlint or workflow YAML parse>.
- Not verified: <honest list of platform, hardware, or scenario gaps>.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
```

Windows releases publish both NSIS (`setup.exe`) and MSI assets. Keep the updater manifest split
by installer type: `windows-x86_64-nsis` must point at the NSIS asset, `windows-x86_64-msi` must
point at the MSI asset, and the generic `windows-x86_64` fallback should stay on NSIS because
fresh installs are documented as NSIS installs. Mixing installer types during an update creates
separate Windows Installed Apps entries.

## Updater Signing Key Rotation

Tauri's updater verifies update packages with the public key baked into the installed app at
build time (`src-tauri/tauri.conf.json` → `plugins.updater.pubkey`). The GitHub release workflow
signs updater packages with `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the published channel manifest then carries the generated
`.sig` file contents as each platform's `signature`.

Never switch the CI signing secret and the baked public key in the same release. The safe rotation
is a two-release sequence:

1. **Transition release N:** the release commit contains the **new** public key in
   `src-tauri/tauri.conf.json`, but CI still uses the **old** `TAURI_SIGNING_PRIVATE_KEY`. Existing
   installs trust only the old baked key, so they can verify and apply this release. After applying
   it, those installs contain the new baked public key.
2. **New-key release N+1:** only after release N has shipped and Guus has explicitly decided it is
   safe to advance, update CI out of band so `TAURI_SIGNING_PRIVATE_KEY` contains the new private
   key and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` contains its passphrase. Release N+1 and later are
   signed with the new key and verified by installs that already applied N.

For the task-1240 rotation, the transition commit bakes in the new updater public key with minisign
fingerprint `C6FADFD59D732197`. The old pre-launch key baked into previously released builds had
fingerprint `545D7BA77EDEA7E1`. The replacement private key and passphrase must be delivered to
Guus out of band; do not commit them and do not paste them into release notes, issues, PRs, logs,
or chat. The CI secret change is deliberately not part of this commit.

### Current state (2026-09-25): release N shipped, N+1 never cut

The transition release N was **0.3.0** (commit `1fe52ac`/`fc7395d`, 2026-07-25): it was the first
release to bake `C6FADFD59D732197`. N+1 was never cut, so CI kept signing **every** release from
0.3.0 through 0.8.3 with the old key `545D7BA77EDEA7E1`. Consequences, verified against the
published artifacts and manifests:

- Every install on 0.3.0-0.8.x trusts only `C6FADFD59D732197` and rejects every published
  update. `node scripts/check-updater-signature-key.mjs --manifest https://releases.beebeeb.io/desktop/latest.json`
  reports 4 of 4 signatures on `545D7BA77EDEA7E1` (same for `beta.json` and `alpha.json`).
- 0.8.3 (the sync-daemon IPC privilege-escalation fix) therefore cannot reach any 0.3.0+ install
  in-app, whatever channel it is promoted to.
- Only pre-0.3.0 installs (which trust `545D7BA77EDEA7E1`) can still apply a 545D-signed update.

The way out is to cut N+1 now (checklist below). After it, 0.3.0+ installs update normally;
pre-0.3.0 installs will reject N+1 and need one manual reinstall. The N+1 release notes and the
README must say that in plain words.

### Signature-key guard

`scripts/check-updater-signature-key.mjs` decodes the minisign key ID of each updater signature
and compares it with the key ID of `plugins.updater.pubkey` in `tauri.conf.json`. The release
workflow runs it twice and fails closed on any mismatch:

- in the build job, on every staged `release-assets/*.sig` (before anything is published), and
- in `publish-manifest`, on the generated channel manifest (this also covers
  `publish_existing=true` promotions). Tags that predate the guard cannot be promoted without the
  transition flag.

A transition release N is the only legitimate mismatch. For that release alone, trigger the
workflow with `updater_key_transition=true`; the guard then reports the mismatch as a warning.
Never set it for any other release.

Mechanical checklist for release N:

1. Confirm `src-tauri/tauri.conf.json` contains the new public key fingerprint
   `C6FADFD59D732197`.
2. Confirm repository/organization secrets still point at the old updater signing key. Do not
   update `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` yet.
3. Run the normal release workflow for release N with `updater_key_transition=true`. Its updater
   packages must be signed by the old key, and the resulting manifest signatures must be the
   old-key signatures generated by that run.
4. Smoke-test updating an existing install built with the old public key to release N.
5. Record evidence before planning N+1.

Mechanical checklist for release N+1:

1. Do not start N+1 until Guus has confirmed release N adoption/recovery expectations. With the
   current static `releases.beebeeb.io/desktop/latest.json` feed, an old install that first checks
   after N+1 is published will see the new-key signature and reject it; public rollout needs either
   confidence that old installs have crossed N, an accepted manual-reinstall fallback, or a
   version-aware update feed that can keep serving N to old clients.
2. Confirm the N+1 release commit still contains the new public key fingerprint
   `C6FADFD59D732197`.
3. Update CI secrets out of band: `TAURI_SIGNING_PRIVATE_KEY` is the new private key contents, and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is the new passphrase.
4. Run the normal release workflow for release N+1 with `updater_key_transition=false` (the
   default). If the CI secret still holds the old key, the signature-key guard fails the build job
   before any asset is published. The RELEASE_NOTES.md for N+1 must say that installs older than
   release N need a manual reinstall from the GitHub release.
5. Smoke-test updating an install already on release N to release N+1, and confirm
   `node scripts/check-updater-signature-key.mjs --manifest https://releases.beebeeb.io/desktop/<channel>.json`
   passes for every channel N+1 was published to.
6. Update the public verification key on beebeeb.io/download: in the site repo, `src/lib/downloads.ts`
   (`DESKTOP_MINISIGN_KEY_ID` / `DESKTOP_MINISIGN_PUBLIC_KEY`) still names the pre-rotation key
   `545D7BA77EDEA7E1` that signs release N's assets. After N+1 is published, switch it to the new key
   `C6FADFD59D732197` and prove it with a real `minisign -Vm <asset> -P <key>` against an N+1 asset
   before deploying the site. Also update the key ID in this repo's README.

Run the local verifier before either release. It requires the Tauri CLI from `bun install`; in a
worktree without `node_modules`, set `TAURI_CLI` to the installed `@tauri-apps/cli/tauri.js`.

```sh
node scripts/verify-updater-key-rotation.mjs
```

## Build Once, Promote By Manifest

Desktop release artifacts are built once with a plain semver version such as `0.2.1`. Do not bake
`-alpha` or `-beta` into the release version. The release channel is selected separately in the
workflow and is represented only by the channel manifest that points at the already-built assets:

- `alpha` updates `desktop/alpha.json`
- `beta` updates `desktop/beta.json`
- `stable` updates `desktop/latest.json`

The app records the channel manifest that actually served the installed update in
`DesktopConfig.installed_release_channel`. That is the "current channel" shown in About and in
downgrade messaging. `DesktopConfig.release_channel` remains only the user's configured channel to
check next. Switching the configured channel does not rewrite the current-channel display until an
update or downgrade is actually installed from that channel.

## Procedure

1. Author `RELEASE_NOTES.md` at the repository root for the plain semver version being cut, such as `0.2.1`.
2. Commit `RELEASE_NOTES.md` with the release preparation changes.
3. Trigger `.github/workflows/release.yml` with `version=<plain semver>`, `channel=<initial channel>`, and `publish_existing=false`.
4. The workflow validates the notes, then runs the release test gate on `ubuntu-latest` (`bun test` for the frontend suite and `cargo test --locked` from `src-tauri` for the Rust crate). It fails closed before the Windows/Linux build matrix if `RELEASE_NOTES.md` is missing, the notes do not mention the exact version string, or either test command fails — no installer artifacts are produced until the gate is green.
5. Once the gate passes, the workflow runs the Windows/Linux build matrix once, creates `desktop-v<version>`, uploads the assets, then publishes only the selected channel manifest.
6. Promote the same build to another channel by rerunning `.github/workflows/release.yml` with the same `version`, the new `channel`, and `publish_existing=true`.
7. A `publish_existing=true` run skips release-note validation, skips the test gate, skips the build matrix, skips GitHub release creation, and only rewrites the selected channel manifest to point at the existing `desktop-v<version>` assets.
8. The `publish-manifest` job reads the GitHub release body into the channel manifest `notes` field, so the authored notes are what the in-app updater shows on every promoted channel.

## Style Notes

- Be specific about what changed and what was verified.
- Prefer short, concrete bullets over broad summaries.
- Keep the verification section honest: include test counts, hardware used, and anything important that was not verified.
