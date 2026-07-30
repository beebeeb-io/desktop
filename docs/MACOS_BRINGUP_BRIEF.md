# Desktop macOS bring-up — what needs to happen next

Written 2026-07-30 by the desktop lead session, at Guus's explicit request, for whichever
Claude session next has a real Mac available to pick this up. macOS is currently deliberately
parked — not because the architecture doesn't work, but because every remaining blocker needs
either a physical Mac, an Apple Developer credential, or an interactive Apple ID login, none of
which are available in the Linux/WSL2 environment this workspace normally runs in.

**The good news first:** this is not a "does it work at all" problem. A development-signed build
has already been installed on real Mac hardware, the File Provider domain registered, and Finder
showed the real `~/Library/CloudStorage/Beebeeb-Drive` mount with live namespaces — confirmed with
real logs, not just source review (see task `0351`'s verification evidence, 2026-05-18/22). The
remaining gaps are specific, enumerable, and mostly about **credentials and CI automation for
distribution**, not unsolved product problems.

## Current release-matrix state

`release.yml`'s build matrix has macOS **commented out, not deleted** — re-enabling it is a
one-line change once the blockers below are cleared. Windows and Linux ship every release; macOS
has shipped zero releases.

## Blockers, in the order to tackle them

### 1. Developer ID + notarization credentials (blocks: real release builds, task `0342`)

No `Developer ID Application` signing identity and no App Store Connect API key /
notary-credentials exist anywhere this workspace's CI or any prior session could reach.
`security find-identity -v -p codesigning` on the one Mac that was used only ever showed `Apple
Development` (a *development*-tier identity, fine for local testing, useless for a distributable
release) and an unrelated `iPhone Distribution` identity for a different bundle.

**What's needed:** Guus needs to either generate a `Developer ID Application` certificate + export
it (as a `.p12` + password) from his own Apple Developer account, or provision an App Store
Connect API key with notary permissions — either path requires his Apple ID and 2FA device, so
this cannot be automated by any agent without his direct involvement at least once. Once obtained,
these become CI secrets (`repos/desktop/.github/workflows/release.yml` already has the plumbing
for macOS bundle inputs staged from base64 CI secrets — task `0351`'s notes confirm the release
workflow already prepares provisioning-profile inputs this way for the containing app + `.appex`,
it just has nothing real to consume yet).

### 2. macOS provisioning profiles for the containing app + File Provider extension

Separate from signing identity: the app (`io.beebeeb.app`) and its File Provider extension
(`io.beebeeb.app.FileProvider`) each need a **macOS** provisioning profile (not iOS/visionOS —
task `0351` found the previously-downloaded profiles under
`~/Library/MobileDevice/Provisioning Profiles` were the wrong platform entirely), both carrying
the app group `R8352WDJJR.io.beebeeb.app.fileprovider`. A **development**-tier version of these
was successfully obtained via `fastlane sigh --platform macos --development` after registering the
Mac as a development device and completing interactive Apple 2FA (task `0351`, 2026-05-18 23:12) —
this is what proved the whole pipeline works end to end. A **distribution**-tier equivalent is
needed for anything that ships to users; `fastlane sigh download_all --platform macos` in
non-interactive/CI mode failed outright with "Missing username" and then hit 2FA — this genuinely
cannot be scripted without either an App Store Connect API key (preferred, no 2FA prompt) or a Mac
with an already-authenticated Apple ID session.

### 3. Universal (arm64 + x86_64) build support

`scripts/build-fileprovider-extension.sh`'s architecture detection does not handle
`TAURI_ENV_ARCH=universal` — a real universal release build needs the File Provider extension
built separately for both architectures and combined with `lipo`. Right now the script only knows
how to build for a single, explicitly-named arch. This is a self-contained shell-script fix, doable
without Apple credentials — the only reason it hasn't been done yet is that until (1) and (2) are
solved, there's no way to verify a universal *signed* build actually works, so fixing this in
isolation would be unverified work. Reasonable to fix and unit-test the arch-detection logic itself
without full signing, but treat "verified" as still gated on a real signed universal build.

### 4. IPC socket peer-credential check has NO macOS implementation — must be done before macOS ships (task 1247, found 2026-07-30)

**This is new since the rest of this brief's history** — found while fixing a P0 security bug in
the Linux desktop build. `src-tauri/src/ipc_socket.rs`'s peer-credential check (`peer_uid`,
gating every connection to the daemon's Unix socket) is implemented for Linux only, using
`SO_PEERCRED`. The non-Linux branch is **deliberately unimplemented and fails closed** (rejects
every connection) — this was a conscious choice, not an oversight: implementing `getpeereid()` (the
BSD/macOS equivalent) blind, with no Mac available to verify the FFI struct/constant layout is
actually correct, risks shipping subtly-wrong `unsafe` security code that looks right but isn't.
**Before macOS's File Provider extension can talk to the daemon over this socket, a macOS session
must:** implement `getpeereid()` (or `LOCAL_PEERCRED`) for the `#[cfg(not(target_os = "linux"))]`
branch in `peer_uid()`, and **verify it on real macOS hardware** — confirm a same-user connection is
accepted and, ideally, confirm (or reason carefully about, since a real cross-UID test needs
root/setuid privileges the way the Linux test also couldn't get) that a different-user connection
is rejected. Do not skip this and just remove the fail-closed guard — that would silently reopen
the exact class of vulnerability task 1247 fixed on Linux, on a platform where it was never closed
to begin with.

### 5. Known remaining product gaps (lower priority, not release-blocking on their own)

- **Secure Enclave / `SecAccessControl` key wrapping** (task `0331`): the master key is currently
  wrapped using plain Keychain password storage under service `io.beebeeb.desktop`, not
  Secure-Enclave-backed access control. Functionally works (lock/unlock, session persistence
  across restart all wired and passing local tests), but doesn't yet use the strongest available
  macOS key-protection primitive. Worth doing before a real release, not a hard blocker to get one
  building.
- **Shared content in Finder** (task `0335`): `Shared with me` namespace surfacing shares in the
  macOS Finder mount is still in-development — desktop's sharing MVP (task 1245, shipped this
  session on Linux via the browse/decrypt path) has NOT been verified through the macOS File
  Provider extension specifically.
- **Recursive pinning/cache** (task `0334`) and **safe uninstall/remount helper** (task `0346`):
  both still in-development, lower-priority polish relative to items 1-4 above.
- **A stale `~/Library/CloudStorage/Beebeeb-Beebeeb` leftover** from an old, pre-rename domain
  display name was observed on the one Mac used for QA (task `0351`, 2026-05-18) — owned by File
  Provider ACLs, resisted direct removal. Likely harmless (a naming migration artifact from
  changing the domain display name from `Beebeeb` to `Drive` mid-development), but worth a clean
  Finder-side check before treating any future macOS QA machine as pristine.

## What a macOS session should NOT do

- Don't attempt to script around Apple 2FA or provisioning-profile downloads non-interactively
  without an App Store Connect API key — every prior attempt (task `0351`) confirmed this fails and
  wastes time; get the API key from Guus first.
- Don't remove or weaken the `#[cfg(not(target_os = "linux"))]` fail-closed peer-cred rejection in
  `ipc_socket.rs` "to get things working" — implement the real macOS equivalent instead (item 4
  above). A daemon that accepts unauthenticated local IPC connections on macOS is exactly the P0
  this session just spent real effort closing on Linux.
- Don't re-attempt the universal-arch build fix in isolation and call it "done" without a real
  signed build to verify against (item 3) — the shell-script logic can be improved and unit-tested
  standalone, but "verified" requires the real thing.
- Don't skip the identifier check: the containing app is `io.beebeeb.app`, the extension is
  `io.beebeeb.app.FileProvider`, the domain id is `io.beebeeb.app.domain`, and the app group is
  `R8352WDJJR.io.beebeeb.app.fileprovider` (this was itself the product of an earlier correction —
  task `0351` originally used `io.beebeeb.desktop`-prefixed identifiers before aligning them to
  what Apple Developer/App Store Connect already had on record).

## Suggested order of operations for the next macOS session

1. Confirm with Guus: does he have (or can he generate) a Developer ID Application certificate and
   an App Store Connect API key with notary access? This single conversation unblocks items 1 and 2
   simultaneously and needs his direct involvement — don't try to work around it.
2. Once credentials exist: re-verify the existing development-signed path still works end to end on
   current `main` (task `0351`'s evidence is from 2026-05-18/22 — the codebase has moved
   significantly since, including all of this session's 0.3.0-0.8.2 roadmap work — a fresh signed
   install + Finder-mount check is warranted before assuming nothing regressed).
3. Swap development signing for the real Developer ID + notarization, get a real signed/notarized
   `.dmg` passing `hdiutil verify` / `codesign --verify` / `spctl --assess` (task `0342`'s
   acceptance criteria).
4. Implement + verify the macOS peer-credential check (item 4) — do this before, or at the latest
   alongside, re-enabling the macOS row in `release.yml`'s build matrix, since that's the point
   real users would start relying on the IPC socket being safe.
5. Fix the universal-arch build script (item 3) once there's a real signed build to test it
   against.
6. Only then: uncomment the macOS entry in `release.yml`'s build matrix and cut the first real
   macOS release.
