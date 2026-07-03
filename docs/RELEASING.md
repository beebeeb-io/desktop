# Desktop Releasing

Desktop releases ship with authored, workbench-style release notes. The generated GitHub changelog still matters, but it is appended after the authored notes instead of being the only release body.

## Release Notes Template

Create `RELEASE_NOTES.md` at the repository root for the exact version being released. Do not keep a reusable placeholder in the repo root; stale notes should make the release workflow fail.

Use this section shape:

```markdown
# Beebeeb Desktop X.Y.Z — <short tagline>

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

## Procedure

1. Author `RELEASE_NOTES.md` at the repository root for the version being cut. The file must contain the exact version string passed to the release workflow, such as `0.1.2-beta.2`; do not use the Windows MSI-safe rewritten version.
2. Commit `RELEASE_NOTES.md` with the release preparation changes.
3. Trigger `.github/workflows/release.yml` with the same semver value.
4. The workflow fails closed before the Windows/Linux build matrix if `RELEASE_NOTES.md` is missing or does not mention the exact version string.
5. The GitHub release body is the authored notes from `RELEASE_NOTES.md` plus the auto-generated changelog appended by GitHub.
6. The `publish-manifest` job reads that release body into the channel manifest `notes` field, so the authored notes are what the in-app updater shows.

## Style Notes

- Be specific about what changed and what was verified.
- Prefer short, concrete bullets over broad summaries.
- Keep the verification section honest: include test counts, hardware used, and anything important that was not verified.
