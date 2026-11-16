# beebeeb-io/desktop

Beebeeb desktop app (PUBLIC repo). Full Rust on Windows and macOS (0.9.0); the React/Tauri app was removed on `v2-native`
(decision 2537: no Tauri build on any OS).
Entrypoints: the native app `native/beebeeb-drive-rune/src/main.rs` (hands over to `shell/`); the engine handle a host calls,
`crates/beebeeb-engine/src/app.rs` (`Engine`); `runner.rs`, `engine_bridge.rs`, `state_db.rs` own scheduling and sync.
Native UI: `native/beebeeb-drive-rune/` (Rune). Capabilities per OS: `docs/CAPABILITIES.md`.
Moved detail (release status, architecture, File Provider rules, macOS window rules, API client, sync engine, signing): `docs/agent-notes.md`.

## Commands

```sh
cd crates/beebeeb-engine && cargo check --locked --all-targets           # compile check (engine)
cd crates/beebeeb-engine && cargo test --locked 2>&1 | tee /tmp/bb-engine-test.log
cd native/beebeeb-drive-rune && cargo test --locked 2>&1 | tee /tmp/bb-native-test.log   # the native app (Rust 1.99.1)
cd crates/beebeeb-updater && cargo test --locked --features net   # the native updater's rules
python3 scripts/assert-cargo-test-counts.py --self-test      # the count guard CI uses (proves it rejects empty runs)
scripts/check-engine-boundary.sh                              # engine must not depend on tauri
```

Quiet checks, from the workspace root: `make test-quiet REPO=desktop` and `make check-quiet REPO=desktop` print one PASS line per target.

## Module map

Module map: `MAP.md` (generated; `make map REPO=desktop`) -- start here for "where does X live"; do not hand-edit.

```
native/beebeeb-drive-rune/  the Rune-based native app (0.9.0): main.rs, shell/ (lifetime, windows, tray), frames/, engine_host/
crates/beebeeb-engine/      the sync engine (no tauri dep); tests/fixtures/ holds the JSON contract fixtures. Under src/:
  app.rs, app/              the `Engine` handle the native app calls (sign-in, unlock, sync start/stop, lists)
  runner.rs, engine_bridge/, state_db.rs   scheduling, bridge to core's sync engine, local state (state.db records the account owner)
  account_binding.rs, own_data_proof.rs    R10: local data belongs to ONE account; every engine start goes through spawn_bound_engine
  config.rs, config/        DesktopConfig: written only via try_update_at_then (one lock, not re-entrant)
  api_client.rs, api_client/direct.rs, direct_download.rs, download_pin.rs   provenance headers, direct S3, version-bound downloads
  keychain.rs, session_commands.rs, sign_out/, signin/, reauth.rs   credentials (OS store), sign-in/out
  windows_cf/               Windows Cloud Files (CFAPI); linux_fuse/ = unmounted prototype
BeebeebFileProvider/        macOS File Provider extension (Swift; ExtensionLog.swift is its only logger); built by scripts/build-fileprovider-extension.sh
packaging/macos/            FileProviderBridge.m (the ObjC bridge), entitlements, icons: the macOS phase's inputs
native/beebeeb-drive-rune/docs/macos/   the macOS phase's carried rules (Finder domain, removal, sign-out, menu bar, packaging and signing)
scripts/                    CI and release tooling (count guards, core pin, engine boundary, release checks)
crates/beebeeb-updater/     the native updater (no Tauri, no UI): manifest, minisign check, policy, one State + step
docs/ (RELEASING.md, CAPABILITIES.md, IPC_PROTOCOL.md, specs/, plans/)
packaging/windows/          the native app's NSIS installer by plain makensis (README.md = the Windows commands and the upgrade rehearsal)
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
- **A Mac never installs an update in place** (App Sandbox, task 2176): "Download update" only; no "Restart to update" on a Mac. Nothing a Mac compiles starts `/usr/bin/open`.
- **No developer word in person-facing Mac copy** (the bun copy guard was removed with the React frontend; the Rust source pins remain); every window opener calls `show_main_window(app, MainNav)`; closing `main` destroys it and the app keeps running.
- **Direct S3 / downloads:** storage links go through the separate storage client only (no default headers, redirects; https); a storage failure is a status number, never reqwest's text (it holds the signed URL);
  a download pins `download_version` (`?object_version_id=`), never invent a pin; `BB_TRANSPORT=api` forces the API.
- **Conflicts never silently drop a version** (default `KeepBoth`, loser renamed `file (Device, HH:MM).ext`); resumable uploads persist in `upload_resume`.
- Identifier `io.beebeeb.app` stays (the `.app` warning is known). Brand: amber rounded square with a lowercase `b`; app name "Beebeeb".

## Never break

The rules whose breakage costs data, a key, a credential or a user. One line each; the detail is in `docs/agent-notes.md`.

- macOS builds locally, not in CI (Developer ID key never exported — task 1608). The macOS release steps are carried in
  `native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md` (the 0.8.x script went with the Tauri app).
- The updater private key `~/.tauri/beebeeb-desktop.key` is mode 0600, **never commit**; generated without a password, so rotate before public release.
- File Provider logs only through `ExtensionLog`, never `NSLog` (on macOS 26 it reaches the unified log as `<private>`); never a path, file name or error text.
- Finder blocker is provisioning: fix signing/provisioning — never mask as a UI timeout or a fake local folder root.
- Every domain removal passes `NSFileProviderDomainRemovalModePreserveDirtyUserData`; never add plain `removeDomain:`/`remove(domain)`/`removeAllDomains`.
- Settings writes (`set_desktop_config`) are merge-only — send only what changed, never the whole object read on open.
- Sign-out keep step (1887): copy every waiting save with `clonefile` (never overwriting) and records it durably BEFORE the purge.
- Finder setup: sign-out removes + holds; a revoked session keeps; Lock cancels, never removes. Surfaces read `finder_setup_state` + `finder-setup-changed`.
- "Open in Finder": the gate is never held across the open; a Mac never asks the File Provider about a shared-with-me row (ruling 1701: webapp-only).
- Sign-out server-revoke reports a status word, never the token or reqwest's text, and never changes the sign-out's result (task 1886).
- R10: changes the engine cannot tie to an own account are held, never sent or deleted until the person decides in Settings › Sync.
- `vault_unlock_route` fails closed when the Keychain cannot be read (nothing is written over a key that may be there); never part of the 1 Hz `sync_status`.
- A null `download_version` (legacy) keeps the unpinned download; never invent a pin. Windows Cloud Files roots are PROGRESSIVE (LD-0105 W1); a placeholder's bytes are trusted only for its recorded minted version (task 2180), else it is poisoned and emptied (S6), never written into.
- Finder writes (`write_id`): record the completion first; the op is never abandoned/parked — it retries only local landing (one transaction).
- A sync-folder create, truncate, replace, rename, copy, upload-from or delete by a name the folder did not report asks `name_alias::conflict_below` (core's `short_name_alias`) first, and takes its conflict path on a conflict: on Windows the name can reach another file through its 8.3 short name (task 2407).
- A per-app name (bundle id, config/state folder, credential service, sync folder, Cloud Files provider name/GUID/class ids, backup device, daemon socket, app group, log folder) comes from the `EngineIdentity` the code is handed (`crates/beebeeb-engine/src/identity.rs`: `REAL` and `TEST`, nothing else): never a literal, never `identity::REAL` outside the two identity files (`crates/beebeeb-engine/src/identity.rs` and the native `shell/identity.rs`) and the native build script (`identity_pin_tests.rs`). A native build is the TEST app unless built with `BEEBEEB_APP_IDENTIFIER=io.beebeeb.app`.
- A storage location is a COUNTRY, never a city (Guus, 2026-10-19).
- The Backups root is the row flagged `system_role = backups_root`, never a folder found by name; everything at or under it, and a move into it, is refused BEFORE a Windows delete/rename and by Finder's capabilities (core's `check_drive_mutation` on its `BackupPosition`; an item created in the tree is not refused), answered from the state DB only (never the network, never this process's own change); its files are read-only (Windows `FILE_ATTRIBUTE_READONLY`, which the engine's own changes lift through `read_only::Lifted` only; Finder without `allowsWriting`); a device's `.device.json` is never listed; `docs/CAPABILITIES.md` (LD-0113).

## Design files

`design/hifi/` holds THREE live design sources and nothing else: `macos-menubar-popover.html`, `macos-settings-dialogs.html` and `macos-two-surface.html`. They are desktop's only copies, the design source of truth that the macOS specs and plans cite. All other design files live in the workspace root `../../design/hifi/` -- never copy them into this public repo.
