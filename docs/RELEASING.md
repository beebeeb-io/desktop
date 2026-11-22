# Desktop Releasing

Desktop releases ship with authored, workbench-style release notes. The generated GitHub changelog still matters, but it is appended after the authored notes instead of being the only release body.

**Two release lines.** The native app (0.9.0 and later) is released by `.github/workflows/release-native-windows.yml`
("Native Windows releases (0.9.0 and later)" below). The 0.8.x Tauri app is released from `main` with that line's own
`release.yml` until `v2-native` merges into `main`; on `v2-native` the Tauri app, `release.yml`, the macOS signing
workflow and the macOS release scripts are gone (no Tauri build on any OS in 0.9.0). Sections that describe only the
0.8.x line say so; that line's full copy of this file is on `main` and at tag `desktop-v0.8.25`.

## Release blockers

`RELEASE_BLOCKERS.md` at the repository root lists what must be fixed before any desktop release is cut, one list item (a line that starts with `- `) per blocker. While it lists an item, the release checks refuse: on this line `release-native-windows.yml` fails closed in its `validate-release-notes` job (step "Refuse while RELEASE_BLOCKERS.md lists a Windows item", with an `::error` that names the file). On the 0.8.x line the same refusal lives in that line's `release.yml` and `scripts/release-macos-local.sh`. A blocker is added when a merge to main ships something that must not reach a release yet, and the change that resolves it removes its item; an empty list, or no file, lets the release through. It exists because a rule that lives only in someone's memory does not stop a release: the first blocker is that a sign-out discards Finder saves still waiting to upload, so no release may ship the Finder write queue until that is fixed. On the 0.8.x line `scripts/test-release-macos-local.sh` has a scenario that proves the macOS script refuses. An item prefixed `- [macOS] ` (exact token, case-sensitive) is scoped to macOS only and is skipped by the Windows and Linux release workflow; `scripts/release-blockers-check.sh` implements the scope logic, and `scripts/test-release-blockers-check.sh` covers it.

**Comparison with main (LD-0014).** A release branch carries its own copy of `RELEASE_BLOCKERS.md`, so deleting an item there would lift a gate that main still holds. `scripts/verify-release-branch-point.sh` (run for `release/*` dispatches before the build) therefore also refuses a branch whose file lacks an item that `origin/main`'s file lists, of any platform, and names the missing items. `scripts/release-blockers-check.sh` does the comparison when `RELEASE_BLOCKERS_MAIN_REF` is set (items are compared after trimming whitespace and normalising the bullet; a missing item is printed as `main: <item>` and binds by its scope). The reverse is allowed: an item the branch has and main no longer lists still binds. Resolve a blocker on main first, then bring that change into the release branch.

**Amended 2026-10-24:** that first item was removed for desktop 0.8.12 on a maintainer waiver after 1 of the 12 sign-out device checks had run and passed (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`, §13.4 and §14). It goes back into the file after 0.8.12, so later releases stay gated until the other 11 pass.

**Amended 2026-11-05:** the unscoped shared-folders item (2237d, Task 13's signed Finder/Explorer device rung) was moved out of this file before desktop 0.8.21. The maintainers ruled that it gates turning `shared_folders_v1` on for everyone (step 6 of `docs/superpowers/plans/2026-10-29-2237-flip-sequence-and-device-rungs.md` in the workspace), not a desktop release. The rung runs on released builds against production once the flag is on for the founders only (step 5), so it cannot also gate the release those builds come from. 0.8.21 ships with the flag off: on 2026-11-05 production's `shared_folders_v1` was disabled with 0 rules.

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

<!-- Pick the line that matches the channel you pass to the workflow. The release body is
     written once and reused unchanged on every later promotion, so it must stay true on
     the first channel. -->
<!-- stable: --> Existing Windows installs receive this release through the in-app updater automatically.
<!-- alpha/beta: --> This release is published to the <alpha|beta> channel first. Windows installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

<!-- 0.8.x only (a native release is Windows-only, with no .dmg): -->
On macOS the in-app update cannot install: download the `.dmg` from this release and drag Beebeeb into Applications to replace the installed version.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets; <!-- 0.8.x only: --> for macOS, the `.dmg` from the same release.

<!-- changelog
title: Desktop X.Y.Z: <short tagline>
date: YYYY-MM-DD
tags: desktop
release: https://github.com/beebeeb-io/desktop/releases/tag/desktop-vX.Y.Z
summary: <one plain sentence for the website changelog>
new:
- <user-visible addition, one line each>
improved:
- <user-visible improvement, one line each>
-->
```

### Changelog step (every release)

The website changelog entry is not the generated GitHub changelog (`generate_release_notes: true` only appends that
under the authored notes). It is written by hand as the hidden `<!-- changelog ... -->` block at the **end** of
`RELEASE_NOTES.md` (shape above; a filled example is in the repository's `RELEASE_NOTES.md`). The step, in order:

1. Author the block with the notes, in plain user-facing language: what a customer sees, not what changed in code.
2. Commit it with the notes. The `release-native-windows.yml` job `publish-release` fails closed, with an `::error`
   naming the file, when the block is missing; that is late (after the build), so check for it before dispatching.
3. After the release is published, the site team turns the block into the website entry from the site repo:
   `bun scripts/changelog-publish.ts --release desktop desktop-v<version>` (dry run: validates and prints), then
   `--create` for a draft or `--publish`. Never publish the entry before the GitHub release is published.

The release workflow enforces these rules (`node scripts/release-channel-checks.mjs notes <version> <channel>`,
tested in `tests/releaseChannelChecks.test.ts`):

- **Channel.** When `channel` is `alpha` or `beta`, notes that promise automatic delivery to existing installs
  fail. desktop-v0.8.3, a security fix, shipped with the unconditional stable wording while it was only ever
  published to `alpha.json`.
- **macOS (task 2176), on every channel.** On macOS the in-app update cannot install, because the App Sandbox denies
  replacing the bundle. desktop-v0.8.13 to v0.8.20 (8 releases) still said "Existing desktop installs receive this
  release through the in-app updater automatically", which was false for every Mac. So:
  - The `### Install / Update` section must exist and must contain the macOS line above **word for word**
    (`MAC_INSTALL_LINE` in the script; hard wraps are fine). The line is true on every channel and for every
    installed build. Builds up to 0.8.20 offer "Restart to update", which fails under the sandbox. Later builds
    offer a download on stable and the release page on alpha/beta (`src/macUpdateOffer.ts`).
  - Any other sentence in that section about updating, the updater or automatic delivery must name Windows and
    must not name macOS. A Linux-scoped sentence fails on purpose, until in-app delivery on Linux is verified.
- **Windows + Linux only release (when `[macOS]` blockers bind).** When `RELEASE_BLOCKERS.md`
  lists one or more `[macOS]`-scoped items at the release ref, the version is a Windows + Linux
  release; macOS follows as a later separate version and is never re-cut from the same tag.
  In that case the notes validator does **not** require the macOS Install line. Instead it requires
  the line:

  > macOS: not in this release; it follows in a later version.

  (`MACOS_NOT_IN_RELEASE_LINE` in `scripts/release-channel-checks.mjs`.) The 0.8.x `release.yml`
  workflow detects the binding `[macOS]` items automatically and passes `--no-macos` to the
  validator; `release-native-windows.yml` always passes it (a native release is Windows only).
  `scripts/test-release-blockers-check.sh` and `scripts/tests/releaseChannelChecks.test.ts`
  cover both branches. In both modes a sentence about updating or automatic delivery must name Windows and
  not macOS (LD-0077); only the macOS line and the not-in-this-release line are exempt.

On the 0.8.x line, Windows releases publish both NSIS (`setup.exe`) and MSI assets (a native 0.9.0 release publishes one NSIS installer and no MSI). Keep the updater manifest split
by installer type: `windows-x86_64-nsis` must point at the NSIS asset, `windows-x86_64-msi` must
point at the MSI asset, and the generic `windows-x86_64` fallback should stay on NSIS because
fresh installs are documented as NSIS installs. Mixing installer types during an update creates
separate Windows Installed Apps entries.

## Updater Signing Key Rotation (0.8.x line)

This section is the 0.8.x line's. The native line's key is its own (`crates/beebeeb-updater/keys/updater-v2.pub`;
"Native Windows releases" below).

Tauri's updater verifies update packages with the public key baked into the installed 0.8.x app at
build time (that app's `tauri.conf.json` → `plugins.updater.pubkey`; this repo keeps the same key as
`crates/beebeeb-updater/keys/updater-0.8.pub`). The GitHub release workflow
signs updater packages with `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the published channel manifest then carries the generated
`.sig` file contents as each platform's `signature`.

Never switch the CI signing secret and the baked public key in the same release. The safe rotation
is a two-release sequence:

1. **Transition release N:** the release commit contains the **new** public key in
   the app's `tauri.conf.json`, but CI still uses the **old** `TAURI_SIGNING_PRIVATE_KEY`. Existing
   installs trust only the old baked key, so they can verify and apply this release. After applying
   it, those installs contain the new baked public key.
2. **New-key release N+1:** only after release N has shipped and a maintainer has explicitly decided it is
   safe to advance, update CI out of band so `TAURI_SIGNING_PRIVATE_KEY` contains the new private
   key and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` contains its passphrase. Release N+1 and later are
   signed with the new key and verified by installs that already applied N.

For the task-1240 rotation, the transition commit bakes in the new updater public key with minisign
fingerprint `C6FADFD59D732197`. The old pre-launch key baked into previously released builds had
fingerprint `545D7BA77EDEA7E1`. The replacement private key and passphrase must be delivered to
a maintainer out of band; do not commit them and do not paste them into release notes, issues, PRs, logs,
or chat. The CI secret change is deliberately not part of this commit.

### State of the task-1240 rotation (checked 2026-10-29): done

- **Release N was 0.3.0** (commit `1fe52ac`/`fc7395d`, 2026-07-25), the first to bake
  `C6FADFD59D732197`. N+1 was not cut then, so CI kept signing every release from 0.3.0 through
  **0.8.3** with the old key `545D7BA77EDEA7E1` (0.8.3: 5 of 5 `.sig` files on `545D`). In that
  time no 0.3.0+ install could apply an update in-app.
- **Release N+1 was 0.8.4** (#62, 2026-09-25), the first signed with `C6FADFD59D732197`. Every
  release since is signed with it: 0.8.4 through 0.8.20, 17 releases, every updater `.sig` on the
  release (5 or 6 each) passes `node scripts/check-updater-signature-key.mjs --sig <files>`. The
  `latest`, `beta` and `alpha` manifests (all 0.8.20) pass `--manifest`, 5 of 5 each.
- **Who can update in-app:** every install on 0.3.0 or later. An install older than 0.3.0 trusts
  only `545D7BA77EDEA7E1`, rejects every release since 0.8.4, and needs one manual reinstall from
  the GitHub release or beebeeb.io/download.
- **The published verification key** is the new one: the site's `src/lib/downloads.ts` names
  `C6FADFD59D732197` (N+1 checklist step 6).

The two checklists below stay for the next rotation.

### Signature-key guard

`scripts/check-updater-signature-key.mjs` decodes the minisign key ID of each updater signature
and compares it with the key ID of the 0.8.x key (`crates/beebeeb-updater/keys/updater-0.8.pub`) by
default, or of the key named by `--pubkey-file` (the native line passes `updater-v2.pub`). The release
workflow runs it twice and fails closed on any mismatch:

- in the build job, on every staged `release-assets/*.sig` (before anything is published), and
- in `publish-manifest`, on the generated channel manifest (this also covers
  `publish_existing=true` promotions). That step runs the checker from the release tag, so a tag
  cut before the guard was merged (0.8.20 and every older tag) has no checker, and the step
  refuses it.

**Promoting a tag cut before the guard** (for example 0.8.20 to another channel) needs
`updater_key_transition=true`, and then the manifest step only warns: nothing in the workflow
checks the key. So check it by hand first, with the checker from `main`, and promote only on OK:

```sh
gh release download desktop-v<version> -R beebeeb-io/desktop --pattern '*.sig' --dir /tmp/sigs-<version>
node scripts/check-updater-signature-key.mjs --sig /tmp/sigs-<version>/*.sig
```

After the promotion, run `--manifest https://releases.beebeeb.io/desktop/<channel>.json` on the
channel it went to. Tags from 0.8.4 on are signed with the baked key; 0.8.3 and older are not, and
must never be promoted.

A transition release N is the only legitimate mismatch. For that release, trigger the workflow
with `updater_key_transition=true`; the guard then reports the mismatch as a warning. The only
other use is promoting a tag cut before the guard, checked by hand as above. Never set it for
anything else.

Mechanical checklist for release N:

1. Confirm the release commit's `tauri.conf.json` contains the new public key fingerprint
   `C6FADFD59D732197`.
2. Confirm repository/organization secrets still point at the old updater signing key. Do not
   update `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` yet.
3. Run the normal release workflow for release N with `updater_key_transition=true`. Its updater
   packages must be signed by the old key, and the resulting manifest signatures must be the
   old-key signatures generated by that run.
4. Smoke-test updating an existing install built with the old public key to release N.
5. Record evidence before planning N+1.

Mechanical checklist for release N+1:

1. Do not start N+1 until a maintainer has confirmed release N adoption/recovery expectations. With the
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

Run the local verifier before either release. It needs the signer's Tauri CLI: run `bun install` in
`scripts/updater-signer/`, then set `TAURI_CLI=scripts/updater-signer/node_modules/@tauri-apps/cli/tauri.js`
(there is no root `package.json`, so the script's default `node_modules` lookup finds nothing).

```sh
node scripts/verify-updater-key-rotation.mjs
```

## Windows Authenticode signing (Azure Artifact Signing)

Since 0.8.8, Windows release artifacts are Authenticode-signed with **Azure Artifact Signing**
(formerly Trusted Signing): public trust, publisher `CN=Initlabs B.V., O=Initlabs B.V., L=Wijchen,
C=NL` (validated organization, KvK 95157565). Before 0.8.8 every Windows installer shipped
unsigned and SmartScreen showed "unknown publisher" (decision 1499, task 1617).

Service coordinates (non-secret):

| Item | Value |
|------|-------|
| Subscription | `devidee-bv` (`b21372e3-1771-4ead-8897-0ba48505e8be`) |
| Resource group / account | `beebeeb-signing` / `beebeebsigning` |
| Endpoint | `https://weu.codesigning.azure.net/` |
| Certificate profile | `beebeeb-publictrust` (PublicTrust; 3-day certs, auto-rotating) |
| CI identity | app registration `beebeeb-desktop-release`, federated credential `repo:beebeeb-io/desktop:environment:desktop-release` (GitHub environment `desktop-release`, deployment branches `main` and `release/*`) |
| Role | `Artifact Signing Certificate Profile Signer`, scoped to the account |

How a 0.8.x release build signs (all in that line's `release.yml`, Windows job only; the native workflow uses the
same Azure account, environment and trust boundary, with its own steps in `release-native-windows.yml` → `build`:
"Azure login for Artifact Signing (OIDC)", "Build and sign the installer", the Authenticode gate):

1. `azure/login` authenticates by GitHub OIDC federated credential — no
   client secret or certificate is stored anywhere. It supplies the
   identifier env vars (`AZURE_CLIENT_ID` / `AZURE_TENANT_ID` /
   `AZURE_SUBSCRIPTION_ID`) and the az CLI login the dlib uses.
   **Timing constraint (measured, release runs 5-7):** the OIDC client
   assertion azure/login relies on is valid ~5 minutes, and the az CLI
   exchanges it once for an access token it caches ~1 hour. The workflow
   therefore runs the ~12-minute cargo compile in a pre-build step BEFORE
   `azure/login`, so tauri build's first sign call happens ~2 minutes after
   login — inside the assertion window — and every later sign rides the
   cached token. Do not move the login step earlier: a compile between login
   and the first sign fails with AADSTS700024 (assertion expired).
2. `scripts/windows/sign-artifact.ps1` is injected as Tauri's
   `bundle.windows.signCommand` at build time (not committed, so local bundling
   keeps working). The bundler invokes it for the app binary before packaging,
   the NSIS installer and its uninstaller, and the MSI. The script runs
   `signtool` with the Microsoft Artifact Signing dlib (fetched from NuGet on
   demand), which authenticates via the ambient `az` login; every signature is
   RFC3161-timestamped (`http://timestamp.acs.microsoft.com` — mandatory, the
   cert rotates every 3 days) and re-verified by the script itself.
3. `scripts/windows/verify-authenticode.ps1` is the fail-closed publication
   gate: every staged installer and the executables *inside* each installer
   payload (NSIS via 7z, MSI via `msiexec /a`) must be `Valid`, signed by
   `CN=Initlabs B.V.`, and timestamped. (Do not also gate on
   `target/.../release/*.exe`: the bundler restores the *unsigned* main binary
   after packaging — the signed app exe exists only inside the payloads.) The
   gate's `-SelfTest` must first reject an unsigned and a wrong-publisher
   fixture. Any failure blocks the release.

Dispatch the release workflow (`release-native-windows.yml`; `release.yml` on the 0.8.x line) from `main` or from a
`release/*` branch. No Azure step is
needed per release. The trust boundary has four parts, and each one is checked:

- **Azure trusts one subject:** `repo:beebeeb-io/desktop:environment:desktop-release`.
  Only a job that names the GitHub environment `desktop-release` gets that subject in
  its OIDC token. In `release-native-windows.yml` those are `build` and `publish-manifest` (in the 0.8.x
  `release.yml`, `build`).
- **The environment deploys only from `main` and `release/*`** (custom deployment
  branch policies; admins cannot bypass them). A run from any other branch is refused
  before the job starts, so it never gets a token Azure accepts.
- **Two repository rulesets protect `release/*`:**
  - **"release branches (create/delete: admin only)":** only a repository admin can
    create or delete a `release/*` branch. A bot, an app or a collaborator without the
    admin role cannot make a branch that is allowed to sign.
  - **"release branches (PR + checks)", with no bypass for anyone, admins included:**
    - every update to an existing release branch goes through a pull request (merge
      method "merge", review threads resolved);
    - the six CI jobs every release branch runs must pass: Frontend, Rust check
      (Windows), Rust check (macOS), Rust test (Linux), File Provider Swift (macOS), and
      the macOS release script check (on `v2-native` five of the six are gone: the Tauri suites,
      Frontend and the macOS release script check; the ruleset needs updating before `v2-native`
      merges into `main`, or a `release/*` pull request waits on checks that never report);
    - force-pushes are refused.
  - A release-branch merge is a plain `gh pr merge --merge` once those checks are
    green. `--admin` is neither needed nor used. A direct push, even with an admin
    token, is refused with "Changes must be made through a pull request".
  - This was proven on 2026-10-29 with a scratch release branch:
    - an admin creation succeeded;
    - a direct admin push was refused;
    - a merge was refused while a required check was red;
    - the same PR merged after the re-run turned green;
    - the scratch branches were then deleted.
  - Until 2026-10-29 a single ruleset blocked every update, and merges needed the
    admin bypass. 0.8.19's #184 and #185 were merged that way, before the rule was
    changed.
  - **What the rulesets do not cover: a branch's starting commit.**
    - Creating a release branch is not a pull request. The required checks are not
      enforced on creation, and CI does not run on pushes to `release/*`.
    - So an admin who creates (or deletes and recreates) `release/x.y.z` at an
      unreviewed commit gets a branch that the release workflow can sign.
    - The release workflow's own test gate still runs on that commit, but nothing reviews it.
    - The rule: cut a release branch only from a `desktop-v*` tag whose commit is the merge
      commit of a pull request merged into `main` or a `release/*` branch, and which has a
      published GitHub release, or from a commit on `main`. Keep the cut commit in the release
      record.
    - A published release alone is not enough: anyone with write access, including a session
      using an admin token, can tag an unreviewed commit and publish a release on it. So the
      tagged commit must also be reviewed.
    - A tag ruleset that limits tagging to the release workflow is not possible: GitHub refuses
      the Actions app as a bypass actor (HTTP 422). That is why the check below tests the tag
      commit itself instead of protecting the tag.
    - The release workflow enforces that rule (task 2281, next bullet): it refuses any other
      branch point before signing.
  - Neither ruleset limits who holds an admin token. A tool or session acting with an
    admin's token can still create, delete or recreate a release branch.
- **The release workflow refuses an unreviewed branch point** (`release-native-windows.yml` here, `release.yml` on
  the 0.8.x line): `verify-branch-point` runs before
  `build`, so before any job holds the `desktop-release` environment or an Azure token. For a
  dispatch from a `release/*` branch it walks the branch's first-parent history from the
  dispatched commit back to its starting point (the first commit that is not a merge of a pull
  request into that branch). Every commit on that walk must be a merge of a PR that was merged
  into this same branch, and the starting point must be the commit of a `desktop-v*` tag with a
  published GitHub release (`draft == false`) whose commit is itself the merge commit of a pull
  request merged into `main` or a `release/*` branch, or a commit on `main`; otherwise the run
  stops before signing and names the offending commit. A tag whose commit is not such a PR merge
  is printed and then treated like any other unreviewed commit: it only passes if it is on
  `main`. `desktop-v0.8.19` is the first tag that meets the tag rule: its commit `db648a6` is the
  merge of #185 into `release/0.8.19`. Older tags whose commit was a direct push (for example
  0.8.18's `e06c55b`) no longer count, which is intended. A dispatch from `main` skips the
  job, because `main` is protected by pull requests. The ruleset
  above cannot gate branch creation, which is why this check exists (task 2281). Branches created
  before that ruleset hold direct commits of their own (for example `release/0.8.19`), so a fresh
  dispatch from them is refused; promoting an existing release with `publish_existing=true` does
  not run the check.

The `desktop-release` environment has 2 required reviewers (see "Signing" in `RELEASE.md`): a release dispatch waits for their approval before any signing step runs.
`signing-login-check.yml` proves the boundary without building or signing anything. It
runs only the Azure login through the environment. From `main` it must pass. From any
other branch it must fail with "not allowed to deploy to desktop-release". Run both
after any change to the environment, the rulesets or the federated credential.

History: until 0.8.18 the credential named one branch (`ref:refs/heads/main`, then one
added per release branch, `release/0.8.13` to `release/0.8.18`). A release branch without
its own credential failed at the Azure login with AADSTS700213 (0.8.18's first run).
0.8.19 was the first release to sign through the environment (2026-10-29). After that,
the seven per-branch credentials were deleted with the maintainer's confirmation, and
only the environment credential remains. The two-way check was run again afterwards:
from `main` it passed, and from another branch it was refused.

SmartScreen honesty: signing removes the "unknown publisher" warning
immediately, but SmartScreen *reputation* accrues with download volume and
starts at zero for a freshly issued certificate (OV and EV are treated the same
since March 2024). A first-download prompt can linger briefly.

Rotation/continuity: certificates are short-lived and rotate inside Azure; the
timestamp keeps every published signature valid indefinitely. Nothing in this
repo or in CI secrets can impersonate the publisher — only that Azure account
plus the GitHub workflow's OIDC subject can.

## macOS

There is no 0.9.0 macOS build yet. The 0.8.x macOS release (a local build on a Mac holding the Developer ID identity,
then a manifest backfill) went with the Tauri app; its steps, checks and acceptance list are carried in
`native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md` for the macOS phase.

## Build Once, Promote By Manifest (0.8.x line)

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

## Procedure (0.8.x line, on `main`)

The native line's procedure is "Native Windows releases (0.9.0 and later)" below.

1. Author `RELEASE_NOTES.md` at the repository root for the plain semver version being cut, such as `0.2.1`.
2. Commit `RELEASE_NOTES.md` with the release preparation changes.
3. Trigger `.github/workflows/release.yml` with `version=<plain semver>`, `channel=<initial channel>`, and `publish_existing=false`.
4. The workflow validates the notes, then runs the release test gate on `ubuntu-latest` (`bun test` for the frontend suite and `cargo test --locked` in the Tauri crate). It fails closed before the Windows/Linux build matrix if `RELEASE_NOTES.md` is missing, the notes do not mention the exact version string, or either test command fails — no installer artifacts are produced until the gate is green.
5. Once the gate passes, the workflow runs the Windows/Linux build matrix once, creates `desktop-v<version>`, uploads the assets, then publishes only the selected channel manifest.
6. Promote the same build to another channel by rerunning `.github/workflows/release.yml` with the same `version`, the new `channel`, and `publish_existing=true`.
7. A `publish_existing=true` run skips release-note validation, skips the test gate, skips the build matrix, skips GitHub release creation, and only rewrites the selected channel manifest to point at the existing `desktop-v<version>` assets.
8. The `publish-manifest` job reads the GitHub release body into the channel manifest `notes` field, so the authored notes are what the in-app updater shows on every promoted channel.
9. The `publish-manifest` job fails closed: every bundle a manifest entry points at, and its `.sig`, must be a fully uploaded, non-empty asset of `desktop-v<version>`, and the `.sig` must download (assets API, up to 3 attempts of at most 120 s) as a Tauri signature whose trusted comment names that bundle (`file:<bundle>`; the signing key is not checked here, so a key-rotation release still publishes). darwin is all-or-nothing: with no macOS asset at all (no dmg, no `Beebeeb.app.tar.gz`, no `.sig`, in any upload state) the manifest omits `darwin-aarch64`; any other macOS state (a dmg without the bundle, an upload still in progress, a missing, empty, unfetchable or foreign `.sig`) fails the channel manifest instead of silently dropping Macs from the update. Tested offline by `scripts/test-release-channel-manifest.sh` (CI job "Release channel manifest (fake gh test)").
10. GitHub's "Latest" release is what beebeeb.io/download offers fresh installs, so it tracks the stable manifest: an initial `alpha`/`beta` release is created with `make_latest: false`, and a `stable` publish (new or `publish_existing=true`) marks `desktop-v<version>` as Latest.
11. After any stable publish, run the delivery drift check: `node scripts/release-channel-checks.mjs delivery`. It is a scripted check, not a judgement call: it compares `https://releases.beebeeb.io/desktop/latest.json` with GitHub's Latest desktop release, prints both values, and exits 1 on drift (or when either cannot be read). `DESKTOP_STABLE_MANIFEST_URL` and `DESKTOP_LATEST_RELEASE_API` override the two endpoints; `scripts/tests/releaseChannelChecks.test.ts` runs it against local servers (match, drift, unreadable). The `delivery-check` job in `release-native-windows.yml` runs it after `mark-latest` on stable; treat a red exit as a failed release.

## Native Windows releases (0.9.0 and later)

The native Windows app has its own workflow, `.github/workflows/release-native-windows.yml`; the 0.8.x line's
`release.yml` (on `main`) is the only writer of `desktop/latest.json`, `beta.json` and `alpha.json`, and it goes when
`v2-native` merges into `main`. Whether the 0.8.25 bridge is promoted to beta and stable from the 0.8.x line before that
merge, so that every 0.8.x install passes through it, is the maintainers' decision. Design and open decisions:
`docs/superpowers/specs/2026-11-11-native-installer-updater.md`. **Status: the workflow has never run.**

- Same inputs (`version`, `channel`, `publish_existing`); a version below 0.9.0 is refused. Same gates: authored
  `RELEASE_NOTES.md` (validated with `--no-macos`: a native release is Windows only), `RELEASE_BLOCKERS.md`, the
  branch-point check, the Authenticode gate (1 installer, no MSI), the updater-signature key guard.
- The installer is built by `packaging/windows/build-installer.ps1 -Mode release` (plain `makensis`), not by the
  Tauri bundler. It is signed for the updater with the same tool as 0.8.x (`tauri signer sign`) and **another
  key**: the native line's own, password-protected v2 key (minisign `FED25848C41F2348`; public half committed as
  `crates/beebeeb-updater/keys/updater-v2.pub`; private half only as the `desktop-release` environment's secrets
  `UPDATER_V2_PRIVATE_KEY` / `UPDATER_V2_PRIVATE_KEY_PASSWORD`). The 0.8.x key signs 0.8.x only. A native build
  trusts the v2 key alone; 0.8.25 trusts both, each for its own line.
- Every job that signs (here `build` and `publish-manifest`; on the 0.8.x line also `release.yml`'s `build` and the
  macOS signing workflow)
  names the `desktop-release` environment, and no step that holds a key installs dependencies. The settings that
  make this a boundary are in the spec, section 6 ("Settings the maintainers must make").
- Promoting a build to another channel and **withdrawing** one are both `publish_existing=true`: a withdrawal
  publishes a newer signed manifest that points at the version to go back to. Never delete a manifest to withdraw
  a build: the host can serve it again.
- A native release is published to the **v2** manifests only: `desktop/v2/{stable,beta,alpha}.json`, each with a
  detached signature `….json.sig` (the native updater refuses an unsigned manifest), and the installer under
  `desktop/artifacts/<version>/`. `scripts/publish-release-files.sh` puts the staged files where
  `releases.beebeeb.io` is served from (repository variable `DESKTOP_RELEASE_PUBLISH_TARGET`; default: the
  `beebeeb-io/releases` checkout). Installs on 0.8.24 or older never see a native release: they first update to
  0.8.25, the bridge (`docs/superpowers/plans/2026-11-11-bridge-0.8.25.md`).
- GitHub "Latest" moves to a native release only when its **stable** manifest is published (job `mark-latest`).

## Style Notes

- Be specific about what changed and what was verified.
- Prefer short, concrete bullets over broad summaries.
- Keep the verification section honest: include test counts, hardware used, and anything important that was not verified.
