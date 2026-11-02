# beebeeb-io/desktop

Beebeeb desktop app (PUBLIC repo). **Tauri v2** shell around the web client (`repos/web`) plus the Rust sync engine from `core`; macOS, Windows, Linux.
Entrypoints: `src/main.tsx` (desktop-owned frontend), `src-tauri/src/lib.rs` (`run()`, command handlers); `runner.rs`, `engine_bridge.rs`, `state_db.rs` own scheduling and sync.
Capabilities per OS: `docs/CAPABILITIES.md`. Moved detail (release status, architecture, File Provider rules, macOS window rules, API client, sync engine, signing): `docs/agent-notes.md`.
Pre-distillation CLAUDE.md verbatim and per-task details: `docs/runbook/desktop-task-history.md`.

## Commands

```sh
bun install
bun run tauri:dev                 # vite on :5173 + native window (loads repos/web on :5173 when it runs)
bun run tauri:build               # .dmg / .msi / .deb / .AppImage
bun test; bunx tsc --noEmit; bun run lint                    # frontend: the CI job
cd src-tauri && cargo check --locked --all-targets           # Rust side without the frontend
cd src-tauri && cargo test --locked 2>&1 | tee /tmp/bb-desktop-test.log
python3 scripts/assert-cargo-test-counts.py --self-test      # the count guard CI uses (proves it rejects empty runs)
scripts/release-macos-local.sh <VERSION> <CHANNEL>           # macOS release: local only, fail-closed (docs/RELEASING.md)
```

Truth lines: `cargo test` → per-binary `test result: ok. N passed; 0 failed` (each library/integration binary must execute tests; only the
`src/main.rs` harness and doctests may be empty); `bun test` → `N pass, 0 fail`; `tauri build … -- --locked` asserts the exit code and bundle files on
disk (`ls` them, paste the listing). A Windows cross-check on Linux is not a native test pass. A release is verified by `RELEASE_NOTES.md` naming the
version AND the workflow run green AND the manifest resolving: three counts, not one adjective. "Cannot be done here" is the normal state
(macOS signing/notary, Windows installers, Finder/File Provider need hardware and credentials): route to `.claude/tasks/decisions/`; never fake a local folder root or a UI timeout to pass a gate.

## Module map

```
src/main.tsx, MacApp.tsx, PopoverApp.tsx   frontend roots: Windows/Linux windows, the Mac `main` window, the menu-bar popover
src/*Model.ts, *Copy.ts, signInFlowStore.ts   pure view models and copy (macCopyGuard scans every person-facing string)
src-tauri/src/lib.rs        run(): Builder + #[tauri::command] handlers (41k lines: outline it, never read it whole); capabilities/default.json = permissions
src-tauri/src/runner.rs, engine_bridge/, state_db.rs   scheduling, bridge to core's sync engine, local state (state.db records the account owner)
src-tauri/src/account_binding.rs, own_data_proof.rs    R10: local data belongs to ONE account; every engine start goes through spawn_bound_engine
src-tauri/src/config.rs     DesktopConfig: written only via try_update_at_then (one lock, not re-entrant)
src-tauri/src/api_client.rs, api_client/direct.rs, direct_download.rs, download_pin.rs   provenance headers, direct S3, version-bound downloads
src-tauri/src/keychain.rs, session_commands.rs, sign_out/, browser_login.rs, reauth.rs   credentials (OS store), sign-in/out
src-tauri/src/finder_setup/, finder_removal.rs, macos_file_provider.rs, macos_popover.rs, surfaces/   macOS Finder + window surfaces
src-tauri/src/windows_cf/   Windows Cloud Files (CFAPI); linux_fuse/ = unmounted prototype
BeebeebFileProvider/        macOS File Provider extension (Swift; ExtensionLog.swift is its only logger); built by scripts/build-fileprovider-extension.sh
src-tauri/macos/FileProviderBridge.m   ObjC bridge; tests/ (bun tests incl. macCopyGuard, macNoInPlaceInstall); scripts/ (release, signing, count guards)
docs/ (RELEASING.md, CAPABILITIES.md, IPC_PROTOCOL.md, specs/, plans/)   linux/, macos/, windows/ = pre-Tauri scaffolds kept for reference
```

## Traps

- **Releases:** follow `docs/RELEASING.md`. The workflow fails closed without a root `RELEASE_NOTES.md` naming the exact semver (and while `RELEASE_BLOCKERS.md` lists
  an item); the notes' `### Install / Update` carries the macOS line from RELEASING.md word for word. macOS builds LOCALLY, never in CI (Developer ID key never exported).
  Windows updater manifests preserve the installer type (NSIS vs MSI). Windows releases before 0.8.8 are unsigned.
- **Updater signing:** the private key `~/.tauri/beebeeb-desktop.key` is never committed; a signature whose key id differs from the baked pubkey fails the release (`scripts/check-updater-signature-key.mjs`).
- **Credentials live in the OS store;** spec 030 never authorizes raw keys in config (Linux fails closed). Cross-platform logic stays in Rust, not duplicated in the frontend.
- **Config writes go through `try_update_at_then`** (`DesktopConfig` has no `save()`); settings writes are merge-only: send only what changed.
- **Local data belongs to one account (R10):** every engine start goes through `spawn_bound_engine`; another account's data resets first (Windows refuses instead).
- **Sign-out:** `clear_session_impl` ends every server session it removes; it keeps waiting Finder saves (task 1887) and refuses with `unsent_changes:` until the person was asked.
- **macOS File Provider:** log only through `ExtensionLog` (never `NSLog`; never a path, file name or error text); every domain removal passes
  `NSFileProviderDomainRemovalModePreserveDirtyUserData` (never plain `removeDomain:`); a Finder blocker is provisioning (app group on both profiles), never masked as a UI timeout.
- **A Mac never installs an update in place** (App Sandbox, task 2176): "Download update" only; no "Restart to update" on a Mac. Nothing a Mac compiles starts `/usr/bin/open`.
- **No developer word in person-facing Mac copy** (`tests/macCopyGuard.test.ts`); every window opener calls `show_main_window(app, MainNav)`; closing `main` destroys it and the app keeps running.
- **Direct S3 / downloads:** storage links go through the separate storage client only (no default headers, redirects; https); a storage failure is a status number, never reqwest's text (it holds the signed URL);
  a download pins `download_version` (`?object_version_id=`), never invent a pin; `BB_TRANSPORT=api` forces the API.
- **Conflicts never silently drop a version** (default `KeepBoth`, loser renamed `file (Device, HH:MM).ext`); resumable uploads persist in `upload_resume`.
- **A storage location is a COUNTRY, never a city** (`tests/noCityInCopy.test.ts` fails on a city name or a `city` read); the desktop keeps no region list.
- Identifier `io.beebeeb.app` stays (the `.app` warning is known). Brand: amber rounded square with a lowercase `b`; app name "Beebeeb".

## How we work

Workspace `CLAUDE.md` → "How we work" is the rule set (evidence, design before code, done, parallel agents). Here: the truth lines above; macOS UI follows the
specs in `docs/specs/` and the design files in `design/` (design wins over code; deviations are recorded before the code changes).
New command: `#[tauri::command]` in `lib.rs`, register in `invoke_handler!`, permission in `capabilities/default.json` if it touches an OS API.

## Graphify and shared docs

Knowledge graph at `graphify-out/`. Architecture question: `graphify query "<question>"` / `graphify explain <symbol>` /
`graphify path "<A>" "<B>"`; overview: `graphify-out/GRAPH_SUMMARY.md` (at most 3k tokens); never read `GRAPH_REPORT.md` whole.
Run `graphify update .` for your own navigation; never commit `graphify-out/` on a feature branch (the lead regenerates it on main).
When you add/change/remove endpoints, types, build commands or dependencies, update the matching skill in the workspace `.claude/skills/`.
