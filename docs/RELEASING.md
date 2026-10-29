# Desktop Releasing

Desktop releases ship with authored, workbench-style release notes. The generated GitHub changelog still matters, but it is appended after the authored notes instead of being the only release body.

## Release blockers

`RELEASE_BLOCKERS.md` at the repository root lists what must be fixed before any desktop release is cut, one list item (a line that starts with `- `) per blocker. While it lists an item, the release checks refuse: `release.yml` fails closed in its "Validate release notes" job (step "Refuse while RELEASE_BLOCKERS.md lists an item", with an `::error` that names the file), and `scripts/release-macos-local.sh` stops at step 1 and prints the items. A blocker is added when a merge to main ships something that must not reach a release yet, and the change that resolves it removes its item; an empty list, or no file, lets the release through. It exists because a rule that lives only in someone's memory does not stop a release: the first blocker is that a sign-out discards Finder saves still waiting to upload, so no release may ship the Finder write queue until that is fixed. `scripts/test-release-macos-local.sh` has a scenario that proves the script refuses.

**Amended 2026-10-24:** that first item was removed for desktop 0.8.12 on a maintainer waiver after 1 of the 12 sign-out device checks had run and passed (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`, §13.4 and §14). It goes back into the file after 0.8.12, so later releases stay gated until the other 11 pass.

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
<!-- stable: --> Existing desktop installs receive this release through the in-app updater automatically.
<!-- alpha/beta: --> This release is published to the <alpha|beta> channel first. Installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
```

The release workflow enforces the channel rule: when `channel` is `alpha` or `beta`, notes that
promise automatic delivery to existing installs fail validation
(`node scripts/release-channel-checks.mjs notes <version> <channel>`, tested in
`tests/releaseChannelChecks.test.ts`). desktop-v0.8.3, a security fix, shipped with the
unconditional stable wording while it was only ever published to `alpha.json`.

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

Mechanical checklist for release N:

1. Confirm `src-tauri/tauri.conf.json` contains the new public key fingerprint
   `C6FADFD59D732197`.
2. Confirm repository/organization secrets still point at the old updater signing key. Do not
   update `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` yet.
3. Run the normal release workflow for release N. Its updater packages must be signed by the old
   key, and the resulting manifest signatures must be the old-key signatures generated by that run.
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
4. Run the normal release workflow for release N+1.
5. Smoke-test updating an install already on release N to release N+1.
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

How a release build signs (all in `release.yml`, Windows job only):

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

Dispatch `release.yml` from `main` or from a `release/*` branch. No Azure step is
needed per release. The trust boundary has four parts, and each one is checked:

- **Azure trusts one subject:** `repo:beebeeb-io/desktop:environment:desktop-release`.
  Only a job that names the GitHub environment `desktop-release` gets that subject in
  its OIDC token. In `release.yml` that is the `build` job.
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
      the macOS release script check;
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
      unreviewed commit gets a branch that `release.yml` can sign.
    - `release.yml`'s own test gate still runs on that commit, but nothing reviews it.
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
    - `release.yml` enforces that rule (task 2281, next bullet): it refuses any other
      branch point before signing.
  - Neither ruleset limits who holds an admin token. A tool or session acting with an
    admin's token can still create, delete or recreate a release branch.
- **`release.yml` refuses an unreviewed branch point:** `verify-branch-point` runs before
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

The workflow has no reviewer gate on the environment: a release stays one dispatch.
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

## macOS: local build, then backfill the manifest

macOS is not in `release.yml`'s CI build matrix (see the matrix comment) — Developer ID signing
happens on a Mac that holds the `Developer ID Application: Devidee B.V. (R8352WDJJR)` keychain
identity, never via an exported `.p12` CI secret. Once Windows and Linux have already been built
and the GitHub release created by the normal workflow run (Procedure below, step 3), the macOS
part is ONE command on an Apple Silicon Mac.

### The one command

```sh
git fetch origin tag desktop-v0.8.7
git worktree add ../desktop-rel-0.8.7 --detach desktop-v0.8.7   # exactly the commit CI built
cd ../desktop-rel-0.8.7
scripts/release-macos-local.sh 0.8.7 alpha --dry-run            # read every command first
scripts/release-macos-local.sh 0.8.7 alpha
```

`scripts/release-macos-local.sh <VERSION> <CHANNEL> [--dry-run] [--from-step N] [--skip-local-sig-check]` runs the 13 steps
below, prints a banner per step, and **stops at the first failure** with the cause, a remediation
and the exact `--from-step N` resume command. Everything that can say "no" runs before the first
upload (step 10); a failure before it leaves GitHub untouched, and the failure message says so.

| Step | What it does | Fail-closed gate |
| --- | --- | --- |
| 1 | Release-state gate (always runs, also on resume) | clean tree; `HEAD` == the commit `desktop-v<VERSION>` points to on origin; the CI release exists with its Windows/Linux `.sig`s; `RELEASE_NOTES.md` names the version, has no `<!-- lead:` marker and carries real test counts (`<N> pass / 0 fail`, `test result: ok. <N> passed`, both non-zero); `RELEASE_BLOCKERS.md` lists no item (see "Release blockers"); `minisign` installed (unless `--skip-local-sig-check`); signing identity, both profiles and the ASC key present; Apple Silicon |
| 2 | Build (`bun install --frozen-lockfile`, version patched into `tauri.conf.json` for the build only, hardened runtime on) with `--bundles app` and a `--config` override turning `createUpdaterArtifacts` off, updater signing variables stripped. Records the commit it built from in `<VERSION>/build-commit` | a resume past this step (`--from-step` 3-13) is refused in step 1 unless that commit equals `HEAD`; `tauri.conf.json` and the tracked `dist/index.html` are restored the moment the build ends and again on any exit; the tree must be clean afterwards |
| 3 | Preflight the unnotarized `.app` | `CFBundleShortVersionString == VERSION`; `macos-release-preflight.sh` (hardened runtime on the app, appex and helper, profiles, entry-point guard). The `spctl` execute assessment is skipped on this pass only (`BB_PREFLIGHT_SKIP_SPCTL_EXECUTE`) because Gatekeeper cannot accept an app Apple has not notarized yet |
| 4 | Notarize the `.app` (`notarytool submit --wait`, ASC API key) | status must be `Accepted`; otherwise the notary log's issues are printed |
| 5 | Staple the `.app`, then verify | `stapler validate`, `codesign --verify --deep --strict`, `spctl -a -vv` must say `source=Notarized Developer ID`, full preflight |
| 6 | Rebuild the `.dmg` from the **stapled** app (`ditto` into a stage folder with an `/Applications` link, `hdiutil create`, `codesign --timestamp`) | the stage copy must validate as stapled; the dmg must verify and show a Developer ID authority |
| 7 | Notarize the `.dmg` | status `Accepted` |
| 8 | Staple the `.dmg`, then verify | `stapler validate`, `spctl -a -vv -t install` must say `source=Notarized Developer ID`, preflight (`hdiutil verify`), and the dmg is mounted read-only to prove the app inside is stapled and versioned |
| 9 | Create the updater `Beebeeb.app.tar.gz` from the stapled app (step 2 builds none: updater artifacts are off there) | single top-level `Beebeeb.app`; the extracted app is stapled and versioned |
| 10 | `gh release upload` dmg + updater bundle to `desktop-v<VERSION>` | both are downloaded again and their sha256 must equal the local files; a stale `.sig` from an earlier attempt is deleted first |
| 11 | Dispatch `sign-macos-updater-artifact.yml`, wait (the run is identified by the URL `gh workflow run` prints; without one, only if exactly one new run exists, otherwise the script stops instead of guessing) | run must succeed; `Beebeeb.app.tar.gz.sig` must exist on the release; verified with `minisign` against the tarball and the baked public key (`BB_UPDATER_VERIFY_PUBKEY` overrides the key for a rotation transition release); fails closed without `minisign` unless `--skip-local-sig-check` |
| 12 | Dispatch `release.yml` (`publish_existing=true`, same version/channel), wait | run must succeed |
| 13 | Read the channel manifest from `beebeeb-io/releases` | `version == VERSION`, `darwin-aarch64` points at this release's `Beebeeb.app.tar.gz`, its signature string equals the uploaded `.sig` (a consistency check only; step 11 is what verifies the signature), Linux/Windows entries still on this tag |

Defaults, all overridable by environment (no secret is stored in the script or the repo):

| Variable | Default |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Devidee B.V. (R8352WDJJR)` (or the SHA-1 hash) |
| `MACOS_APP_PROVISION_PROFILE`, `MACOS_FILE_PROVIDER_PROVISION_PROFILE` | `~/.private_keys/macos-developer-id-profiles/{BeebeebApp,BeebeebFileProvider}.provisionprofile` |
| `BB_NOTARY_KEY_ID`, `BB_NOTARY_ISSUER` | `5KRWK96245`, `8cacf7df-c877-47db-aab2-7c9f1f9f5fda` (App Store Connect API key identifiers, not secrets) |
| `BB_NOTARY_KEY_PATH` | `~/.private_keys/AuthKey_<key id>.p8` |
| `BB_RELEASE_REPO`, `BB_RELEASES_REPO`, `BB_WORKFLOW_REF` | `beebeeb-io/desktop`, `beebeeb-io/releases`, `main` |
| `BB_NOTARY_TIMEOUT`, `BB_POLL_INTERVAL`, `BB_WORKFLOW_TIMEOUT` | `45m`, `15` (s), `1800` (s) |

`--dry-run` prints every command of every step and runs none (not even `git`). `--from-step N`
resumes at step N; step 1 still runs, and each later step fails with a clear message if the
artifacts from the earlier steps are not on disk or were built from another commit than `HEAD` (they live in
`src-tauri/target/release-macos-local/<VERSION>/` and `src-tauri/target/<triple>/release/bundle/`).
Notary submission ids, file hashes and workflow run ids are written to `release-summary.txt` there.

What the script does **not** do, by design: it never commits anything, never touches the stable
manifest unless you pass `stable`, and never replaces installing the dmg on a Mac and exercising
it. A release is verified by `RELEASE_NOTES.md` naming the version AND the workflow runs being
green AND the manifest resolving (step 13), and, for a Finder-facing change, by a hand test.

Step 2 builds only the `.app` (`--bundles app`), so Tauri's `bundle_dmg.sh` never runs, and the dmg is built with `hdiutil`: that script styles the
window through Finder AppleScript and failed with `-10006` (Automation permission) during the
0.8.5 cut. Updater artifacts are switched off for the local build because the updater private key (`TAURI_SIGNING_PRIVATE_KEY`) exists only as a CI secret since the 2026-09-25 rotation, and Tauri exits non-zero when it is asked for a signed updater bundle without it; the file keeps `createUpdaterArtifacts: true` (preflight asserts it, CI needs it) and the off switch is a CLI override. The consequence is a plain volume (the app and an `/Applications` link) without
Tauri's icon positions; compare it with the previous dmg once and say so if that matters.

`scripts/test-release-macos-local.sh` is the Linux-runnable test of this script (shims for the
macOS and GitHub tools; it asserts the step order, the artifact contents and each fail-closed
gate, and is counted: `release-macos-local: N passed, 0 failed`). CI runs it as the job
"macOS release script (shim test)". It does not prove that Apple's tools behave like the shims.

### Manual steps (reference)

The script automates exactly this sequence; keep it as the reference for what each step means and
for running a single step by hand.

1. **Set the app version first** (the script does this for you in step 2; task 1670 issue 3: this step was missing, and a local bundle built without it is versioned `0.1.0`, the placeholder in `src-tauri/tauri.conf.json`, because Tauri names the bundle and reports `CFBundleShortVersionString` from that file's own `version`). It mirrors the `Set app version for this release` step in `.github/workflows/release.yml` (the `jq --arg version "$RELEASE_VERSION" '.version = $version' src-tauri/tauri.conf.json` line, ~L238). Run it in the release worktree, on plain semver only, and never commit the result. Edit `VERSION` to the release version (plain semver, no `-alpha`/`-beta` suffix) and paste the block into your interactive shell. It carries no comments on purpose: macOS's default interactive zsh has `interactive_comments` off, so a trailing `# ...` on the `VERSION=` line is parsed as a command and `VERSION` is never set. It uses if/else, not `|| exit 1`, because `exit` in a pasted block would close your terminal. It works in both zsh and bash. Afterwards `jq -r .version` must print `VERSION`, not `0.1.0`; a bad `VERSION` prints the "plain semver only" line and changes nothing.

   ```sh
   VERSION=0.8.7
   if [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
     jq --arg version "$VERSION" '.version = $version' src-tauri/tauri.conf.json > src-tauri/tauri.conf.json.tmp \
       && mv src-tauri/tauri.conf.json.tmp src-tauri/tauri.conf.json
     jq -r .version src-tauri/tauri.conf.json
   else
     echo "plain semver only: got '$VERSION'" >&2
   fi
   ```

   After the build, `plutil -extract CFBundleShortVersionString raw <built .app>/Contents/Info.plist` must print the same version.
2. Build locally: `APPLE_SIGNING_IDENTITY=<Developer ID hash> MACOS_APP_PROVISION_PROFILE=… MACOS_FILE_PROVIDER_PROVISION_PROFILE=… BEEBEEB_RELEASE_VERSION=<version> BEEBEEB_MACOS_HARDENED_RUNTIME=1 bunx tauri build --target aarch64-apple-darwin --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked` (no updater private key exists on this Mac, so updater artifacts are off and only the `.app` is built; Apple Silicon only for now — see the matrix comment for why). `BEEBEEB_MACOS_HARDENED_RUNTIME=1` is required for a release build: it turns on hardened runtime + secure timestamps on the File Provider extension and its CTL helper, which notarization requires on every nested executable, not just the app.
3. Run `scripts/macos-release-preflight.sh <path to the built .app>` — it fails closed if hardened runtime, the provisioning profiles, or the File Provider entry-point guard (task 1524) are wrong.
4. Notarize the `.app` (zip it first — `ditto -c -k --keepParent Beebeeb.app app.zip`), `xcrun notarytool submit --wait`, then `xcrun stapler staple` it, before building the `.dmg` from the now-stapled app — this way the ticket travels with the app inside the dmg too. Then notarize + staple the `.dmg` itself. `scripts/macos-release-preflight.sh <artifact>` re-run on each proves `spctl -a -vv` / `spctl -a -vv -t install` both say `source=Notarized Developer ID`.
5. Upload the `.dmg` and the updater bundle (made by hand from the stapled app, `tar -czf Beebeeb.app.tar.gz Beebeeb.app` — named from `productName` alone, no version/arch suffix; do not confuse it with the versioned `.dmg` filename) to the existing `desktop-v<version>` GitHub release with `gh release upload`.
6. The updater `.sig` needs `TAURI_SIGNING_PRIVATE_KEY`/`_PASSWORD`, which since the 2026-09-25 rotation (desktop PR #62) exists only as this repo's CI secret. Run `.github/workflows/sign-macos-updater-artifact.yml` via `gh workflow run … -f release_tag=desktop-v<version>` to sign the uploaded `Beebeeb.app.tar.gz` and upload the resulting `.sig` back to the same release — this is the only CI step in the whole macOS route, and it never touches Developer ID material.
7. Re-run `release.yml` with `publish_existing=true` for the same `version`/`channel` — the `publish-manifest` job's `fetch_optional_sig` now finds the macOS `.sig` and backfills the `darwin-aarch64` manifest entry (see that job's comments) without rebuilding or re-tagging Windows/Linux.

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
9. The `publish-manifest` job fails closed: every bundle a manifest entry points at, and its `.sig`, must be a fully uploaded, non-empty asset of `desktop-v<version>`, and the `.sig` must download (assets API, up to 3 attempts of at most 120 s) as a Tauri signature whose trusted comment names that bundle (`file:<bundle>`; the signing key is not checked here, so a key-rotation release still publishes). darwin is all-or-nothing: with no macOS asset at all (no dmg, no `Beebeeb.app.tar.gz`, no `.sig`, in any upload state) the manifest omits `darwin-aarch64`; any other macOS state (a dmg without the bundle, an upload still in progress, a missing, empty, unfetchable or foreign `.sig`) fails the channel manifest instead of silently dropping Macs from the update. Tested offline by `scripts/test-release-channel-manifest.sh` (CI job "Release channel manifest (fake gh test)").
10. GitHub's "Latest" release is what beebeeb.io/download offers fresh installs, so it tracks the stable manifest: an initial `alpha`/`beta` release is created with `make_latest: false`, and a `stable` publish (new or `publish_existing=true`) marks `desktop-v<version>` as Latest.
11. After any stable publish, run `node scripts/release-channel-checks.mjs delivery`. It compares `https://releases.beebeeb.io/desktop/latest.json` with GitHub's Latest desktop release and exits 1 on drift.

## Style Notes

- Be specific about what changed and what was verified.
- Prefer short, concrete bullets over broad summaries.
- Keep the verification section honest: include test counts, hardware used, and anything important that was not verified.
