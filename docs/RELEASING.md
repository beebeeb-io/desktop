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
4. The workflow validates the notes, runs the Windows/Linux build matrix once, creates `desktop-v<version>`, uploads the assets, then publishes only the selected channel manifest.
5. Promote the same build to another channel by rerunning `.github/workflows/release.yml` with the same `version`, the new `channel`, and `publish_existing=true`.
6. A `publish_existing=true` run skips release-note validation, skips the build matrix, skips GitHub release creation, and only rewrites the selected channel manifest to point at the existing `desktop-v<version>` assets.
7. The `publish-manifest` job reads the GitHub release body into the channel manifest `notes` field, so the authored notes are what the in-app updater shows on every promoted channel.

## Style Notes

- Be specific about what changed and what was verified.
- Prefer short, concrete bullets over broad summaries.
- Keep the verification section honest: include test counts, hardware used, and anything important that was not verified.
