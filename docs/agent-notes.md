# Desktop: agent notes (moved detail)

The detail that used to sit in `CLAUDE.md`, moved verbatim by the instruction diet (token plan task 12).
`CLAUDE.md` keeps the commands, the truth lines, the module map and the traps; each section below is
named there by a pointer line. Read the section you need, not the whole file.

Beebeeb desktop app. 0.9.0 is a native Rust app (`native/beebeeb-drive-rune`, a Rune UI, no web view) over the engine
(`crates/beebeeb-engine`) and the Rust sync engine from `core`. The 0.8.x app was a **Tauri v2** shell around the web
client; it is gone in 0.9.0 on every OS. Sections below that describe only the 0.8.x app say so; the full 0.8.x text is
at tag `desktop-v0.8.25`.

> **Correction — 2026-11-16 (replaces the 2026-09-29 note of task 1611):** entrypoints are the native app `native/beebeeb-drive-rune/src/main.rs` and the engine handle `crates/beebeeb-engine/src/app.rs`; the engine's `runner.rs`, `engine_bridge.rs`, `state_db.rs` own scheduling/sync. macOS uses `BeebeebFileProvider/` (built by `scripts/build-fileprovider-extension.sh`), Windows uses the engine's `windows_cf` (CFAPI), Linux FUSE is an unmounted prototype. Legacy native-shell build recipes and Linux online-only claims are historical; spec 030 never authorizes raw keys in config — credentials belong in the OS store (Linux fails closed). See `docs/CAPABILITIES.md`.

## Release status (0.8.x: macOS, Windows, Linux released)

- `desktop-v0.8.5`+ published for all three platforms: MSI + NSIS for Windows, AppImage/.deb/.rpm for Linux, Developer ID signed + notarized `.dmg` for macOS (Apple Silicon only — Intel not built), each with a minisign `.sig` for the auto-updater.
- **Windows signing correction (2026-10-01):** every release through 0.8.7 shipped UNSIGNED; `desktop-v0.8.8` is the first Authenticode-signed Windows release (Azure Artifact Signing, `CN=Initlabs B.V.`, enforced by `scripts/windows/verify-authenticode.ps1`).
- **Windows updater manifests must preserve installer type:** publish `windows-x86_64-nsis` + generic fallback → NSIS asset; `windows-x86_64-msi` → MSI asset (crossing types creates a second Installed Apps entry).
- The 0.8.x `release.yml` (on `main`) runs Windows + Linux only; **macOS builds locally, not in CI** (deliberate: Developer ID key never exported — task 1608). The 0.8.x macOS release script went with the Tauri app; its steps are carried in `native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md`. The native line releases Windows only (`release-native-windows.yml`).
- **Local verification (Linux, 0.8.x):** `bun run tauri build --target x86_64-unknown-linux-gnu -- -- --locked` reproduces build + `.deb`/`.rpm` (needs webkit2gtk-4.1, libayatana-appindicator, patchelf, fuse2/3; throwaway `bunx tauri signer generate` key is fine). `.AppImage` bundling cannot be verified on Arch/WSL2 (vendored linuxdeploy `strip` too old) — CI is the source of truth for that step. Windows/macOS builds cannot be reproduced locally.
- Not yet verified: real install + close-to-tray smoke test on physical Windows hardware.


## Releases

Before cutting a desktop release, follow `docs/RELEASING.md`. The release workflow fails closed
unless the released commit contains a root `RELEASE_NOTES.md` whose contents mention the exact
semver value passed to the workflow; author and commit per-release notes before triggering it. It
also refuses while root `RELEASE_BLOCKERS.md` lists an item. For an `alpha`/`beta` initial channel
it refuses notes that promise automatic delivery to existing installs, and only a `stable` publish
marks the GitHub release Latest. On every channel (task 2176: a Mac cannot install in-app) the
notes' `### Install / Update` section must carry the macOS line from `docs/RELEASING.md` word for
word, and any other update/delivery sentence there must name Windows and not macOS. `node scripts/release-channel-checks.mjs delivery` checks the live
stable manifest against GitHub Latest.


## Architecture, layout, build

The current architecture is in `README.md` → "Architecture"; the module map and the commands are in `CLAUDE.md`. The
0.8.x Tauri app's architecture, layout and `bun run tauri:dev` build are at tag `desktop-v0.8.25`.

## Key facts

- App bundle id `io.beebeeb.app`; File Provider extension id `io.beebeeb.app.FileProvider`; domain id `io.beebeeb.app.domain`; app group `R8352WDJJR.io.beebeeb.app.fileprovider`.
- Cross-platform logic stays in Rust (auth/session, crypto, sync, queueing, conflicts, cache policy, protocol) — not duplicated in the frontend.
- The Tauri warning that `io.beebeeb.app` ends in `.app` is known — keep the identifier unless the owner migrates the App Store Connect record.

## macOS File Provider rules

- **Logging:** the extension logs ONLY through `ExtensionLog` (`BeebeebFileProvider/ExtensionLog.swift`, one `os.Logger`, subsystem `io.beebeeb.app.FileProvider`) — never `NSLog` (on macOS 26 it reaches the unified log as `<private>`). New lines are functions in `ExtensionLog` whose public values are closed (fixed words, bitmasks, UUIDs, NSError domain+code) — never a path, file name or error text. `extension_log` in `crates/beebeeb-engine/tests/file_provider_source_pins.rs` (ported from the 0.8.x `extension_log_pin`) enforces the vocabulary; details: `docs/IPC_PROTOCOL.md` → "Logging".
- **Finder blocker is provisioning:** both containing app and extension profiles must include the exact app group; fix signing/provisioning — never mask as a UI timeout or a fake local folder root.
- **Removing the Finder location keeps un-synced files (task 1882):** every `NSFileProviderManager` domain removal passes `NSFileProviderDomainRemovalModePreserveDirtyUserData`; never add plain `removeDomain:`/`remove(domain)`/`removeAllDomains`. Source pin `test_1882_every_domain_removal_in_the_repo_keeps_unsynced_files` (`crates/beebeeb-engine/tests/file_provider_source_pins.rs`). macOS reports a folder even when it kept nothing — the bridge checks the returned URL (`BeebeebKeptFolderState`) and only surfaces a folder that exists and holds an entry (or is sandbox-blocked), via alert + Settings → Sync row (`kept_unsynced_folder`).
- **Config writes (task 1889):** `DesktopConfig` has no `save()`; the file is written in ONE place, `try_update_at_then`, which holds the one config-write lock in `config.rs` across read/change/write (not re-entrant — nothing inside an update closure calls another update). The ways in: `update_at` (+ `update`, `update_held`, `ensure_account_id`, which are `update_at`), `try_update_at`, `update_at_then`, and `update_at_durable_with` — the one save that flushes its write (the sign-out keep step's, task 1887). Held copies (`HeldConfig`) are read-only. Settings writes (`set_desktop_config`) are merge-only — send only what changed, never the whole object read on open. In-memory pause flag follows the file under the same lock. Pins: `test_1889_*`.
- **A sign-out keeps waiting Finder saves (task 1887, macOS):** `clear_session` gates on an ask snapshot (`sign_out_count`) and refuses with `unsent_changes:` while changes wait and the person was not asked; Settings › Account sends first (`sign_out_send_start`, ≤ 60 s), then asks. On "Sign out" the keep step copies every waiting save into `<container>/Data/Documents/Changes not sent/<date at time>/` with `clonefile` (never overwriting) and records it durably (`kept_unsent_folder`, `kept_unsent_unfinished`) BEFORE the purge, whose guard refuses any deletion unless every waiting write was kept. Windows and Linux unchanged. Spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`; detail: docs/runbook/desktop-task-history.md.
- **Finder setup reconciler** (0.8.x: the Tauri shell's `finder_setup/`; the engine keeps its pure parts in `crates/beebeeb-engine/src/finder_setup/`; the rules are carried in `native/beebeeb-drive-rune/docs/macos/file-provider-domain.md`; spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md`): nobody installs the Finder location on macOS any more; one Rust driver task keeps Beebeeb in Finder (pure core/policy/launch_location + bridge via `macos/FileProviderBridge.m`; NSError-domain+code classification; retries 0/5/15/45 s; triggers: launch, keys arriving, "Try again", userEnabled flip; sign-out removes + holds; Repair removes then checks; revoked session keeps; Lock cancels, never removes). Surfaces read `finder_setup_state` + `finder-setup-changed`; copy in `src/finderSetupCopy.ts`.
- **Launch rule:** `launch_location` classifies `/AppTranslocation/` → translocated; bundle at `/Volumes/<name>/` → disk_image; Applications folder → applications; else elsewhere. `translocated`/`disk_image` report `not_in_applications` and never call add.
- **Lifecycle log:** app view `~/Library/Logs/Beebeeb/lifecycle.log` (sandboxed: `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log`), 1 MB × 3 files, closed vocabulary, redacted NSError text, not a `tracing` sink; holds no user content; "Copy details"/support bundle include the last 200 lines. Whether Console.app lists it is unverified (device check D0 records the answer).
- **"Open in Finder" (task 1885):** nothing a Mac compiles starts `/usr/bin/open` — the bridge keeps the security-scoped URL from `getUserVisibleURLForItemIdentifier` and opens in-process (`beebeeb_open_scoped_url` / `beebeeb_reveal_scoped_url`; GATED resolve then UNGATED open; the gate is never held across the open; scope stopped in the completion handler or 60 s timer). `beebeeb_open_url` opens System Settings (no gate). A Mac never asks the File Provider about a shared-with-me row (ruling 1701: webapp-only). (details: docs/runbook/desktop-task-history.md)
- **Sign-in:** first-run "Sign in with browser" opens `<APP_URL>/cli-auth`; the link carries NO code (person TYPES it — phishing defence); WS init frame adds `client: "desktop"`, version, `os`, hostname; copy in `src/browserLoginCopy.ts`.
- **Re-sign-in in place (R8):** "Sign in again" opens sign-in in place; the same account swaps only its session token (keys, cache, queue, Finder stay). Only an account switch purges (warning with pending-change count → full sign-out → fresh sign-in). A startup 401 drops only the revoked token on macOS/Linux; Windows keeps today's flow.
- **A sign-out ends its server sessions on every OS (task 1886):** `clear_session_impl` (every sign-out, the account switch included) collects every token it takes off the computer in a `DroppedTokens`: the one it begins with (`session_token_for_revoke`, read BEFORE the engine slot is taken; from the slot on nothing touches the session until its clear, which `sign_out_holds_the_engine_slot_until_the_session_is_cleared` pins), and what each clear removes (`clear_session_holding_slot` returns the memory token; `finish_sign_out_turn` / `clear_keychain_session_holding` / Windows `clear_keychain_session_into` read each Keychain layout's token in the same turn as its clear), so a session written while it runs is ended too. `end_dropped_sessions` then runs AFTER the local clear, with the engine slot released: a clear that completed ends every token; one that failed ends only what the computer no longer holds (`session_tokens_held_here`). `POST /api/v1/auth/logout` per token, all within one 3 s bound, best effort: each outcome is a closed `LogoutOutcome` code (`revoked`, `already_ended` for a 401 at info, `refused`+status, `unreachable`, `misconfigured`, `timed_out`, `no_client`), never the token or reqwest's text, and never changes the sign-out's result. Pin `the_sign_out_ends_its_session_on_the_server_on_every_os`; tests `sign_out_revoke_tests`. Not covered: an R8 re-sign-in replaces the old token without revoking it.
- **Local data belongs to one account (R10):** `state.db` records the owner; every engine start goes through `spawn_bound_engine` (`account_binding::bind_before_engine_start` first): same account keeps everything; another account's data or owner-less data resets before anything starts (Windows refuses instead); an incomparable owner stops the start; a failed reset/purge stops. Pin: `every_engine_start_is_bound_first`. On Windows a refusal over owner-less data (or an email-only owner) asks the server in the background (`own_data_proof.rs`, task 2267): it lists this account's whole tree once (snapshot + trash) and judges EVERY My Files row the server named, no sample (in the tree: own when its name decrypts under this session's key; not in the tree: gone only under an own ancestor); a row counts as made on this PC (not judged) only when its version columns are both 0 AND a queued create names it; a name that does not open under this key (unless the node says a file request or a collaborator sealed it; a node that says nothing is no evidence) or a row it cannot place blocks, and names that prove nothing never outnumber own ones. Proven, `StateDb::adopt_owner_holding` records the owner and, in the same transaction, holds every change it cannot tie to an own row, whatever its pause reason, as `found_while_paused` (later changes to those items too, `insert_operation_conn`): never sent or deleted until the person decides in Settings › Sync. A path proves only the create of a folder made here whose own path is an own server folder's (its twin, task 1888), never a new folder inside it; any other change, file content above all, needs one anchor tied to an own row by real parent links, never by a twin or by a folder at a path (also not on the walk up from a row gone from the tree). The card shows them by kind (`found_while_paused_review`: new folders, new files, edits, deletions, changes in folders shared with you; a change anywhere inside a shared folder counts there), and both choices act on exactly the ids it showed, never a change held after that read: "Upload to my Beebeeb" releases them; "Keep on this PC only" (`found_while_paused.rs`) MOVES the items made on this PC they act on out of the sync root into `Beebeeb - not uploaded\YYYY-MM-DD` next to it (same volume, rename only, never over a name, journaled, settled at the next engine start) and forgets their rows. It removes only the staged copies the journal names; one whose live file is gone is copied into the dated folder first, before the rows are forgotten (a restart after that copy does not make it twice). When OneDrive or Dropbox syncs the folder it is made in, the card says so (`synced_by`) instead of "on this PC". Sync starts through `spawn_bound_engine`. Two different user ids are never questioned.
- **Zombie domain correction (1698):** the `DOMAIN_IDENTIFIER` comment about `io.beebeeb.desktop.FileProvider` was wrong (folder is `Beebeeb-Beebeeb`; `getDomains` sees only the calling provider's domains); a pre-rename dev-Mac domain is cleared once by the one-off helper (spec §11), not by the app.

## macOS window rules (task 1683, spec `docs/specs/2026-10-24-macos-two-surface-app.md`)

These were the 0.8.x Tauri app's (one `main` window, the menu-bar popover, `surfaces::launch`). They are carried for the
macOS phase in `native/beebeeb-drive-rune/docs/macos/menu-bar-and-windows.md`; the 0.8.x text is at tag
`desktop-v0.8.25`.

## API client (`crates/beebeeb-engine/src/api_client.rs`)

Every request from the daemon `ApiClient` and every ad-hoc `reqwest::Client` of the engine carries `X-Beebeeb-Client: desktop` + `X-Beebeeb-Client-Version` (`api_client::provenance_headers()`; mirrors CLI's `build_client`). Version sent is `BEEBEEB_RELEASE_VERSION` (release.yml), falling back to `CARGO_PKG_VERSION` — not raw Cargo's `0.1.0`.

**Direct S3 downloads (task 2173):** a whole-file hydrate (`EngineBridge::hydrate_whole_file`) fetches encrypted chunks straight from storage through grant windows when discovery offers `object-download-v1` (`src/direct_download.rs` drives core's `beebeeb_core::direct_transfer` planner; HTTP in `src/api_client/direct.rs`). Rules: discovery cache and storage client live in the `ApiClient` (one signed-in session); `X-Beebeeb-Capabilities` only on the grant POST; storage links through the separate storage client only (no default headers, no redirects, https only unless the API is loopback http); a storage failure is a status number, never reqwest's text (it holds the signed URL); chunk RANGES, thumbnails, versions and uploads stay on the API. `BB_TRANSPORT=api` (wins) or `desktop.toml` `transport = "api"` forces the API path.

**Version-bound downloads (task 1643):** a download takes `download_version` from the metadata it reads (`crates/beebeeb-engine/src/download_pin.rs`) and pins every chunk request to it (`?object_version_id=`), on the API path and the direct path alike; each chunk's wire size and the plaintext totals are checked against the manifest; a 409 or another version is `VersionGone`, one restart from fresh metadata, then an error (nothing written, row `Error`). A null `download_version` (legacy) keeps the unpinned download; never invent a pin. Not covered: Windows Cloud Files PARTIAL hydration across callbacks (task 2180).

## Sync engine

The engine (`crates/beebeeb-engine`) owns the background Rust sync runtime; no protocol is duplicated outside Rust. Conflicts: never silently drop a version — default `KeepBoth` (loser renamed `file (Device, HH:MM).ext`). Watcher debounce 100 ms.

**Upload resume:** `POST /uploads/init` persists the session (`upload_resume` table, keyed by `op_id`, fingerprinted by staged path + size + mtime); watermark advances on every 2xx chunk PUT; retry resumes from the watermark; changed payload / gone session / given-up op abandons (create's orphaned `is_uploading` row is trashed; a replace's never). Exception for macOS Finder writes (`write_id`): after `complete` answers, the completion is recorded first and the op is never abandoned/parked — it retries only local landing (one transaction). Windows local-create completion keeps a separate `upload_finalizations` journal. Advanced settings (`desktop.toml`): `theme` light/dark/system; `local_cache_limit_bytes` per-device cap (`0` = Unlimited; evict LRU non-pinned first, never auto-evict pinned).

**Names that reach another file (task 2407, core 2404):**
- Windows resolves a name through a folder's 8.3 short names too, and strips a trailing dot or space. So a row's path can reach a different existing file: `REPORT~1.PDF` reaches `Report for 2026.pdf`.
- Every site that creates, truncates, replaces, renames, copies, uploads from or deletes a sync-folder entry by a name the folder did not report calls `name_alias::conflict_below(sync_root, path)` first. That is core's `path_safety::short_name_alias`, which reads the entry's own long name and never the name's shape.
- On a conflict, the site takes its conflict path:
  - remote deletion: `Gone`, nothing removed;
  - placeholder minting: row skipped;
  - hydrate, Keep Both and Keep Mine: refused, and the row stays in conflict;
  - keep on this PC: stays held;
  - free up space: kept local.
- A check that cannot finish refuses too.
- `NameConflict` and the check's errors never carry a name.
- Tests stub the answer per path (`name_alias::stub`); the Windows tests prove a real 8.3 alias first and fail under `CI` when the volume makes none.

## Auto-update signing

Both lines verify updates with minisign. The native line (0.9.0 and later) trusts only its own v2 key
(`crates/beebeeb-updater/keys/updater-v2.pub`; the private half only as the `desktop-release` environment's secrets,
`docs/RELEASING.md` → "Native Windows releases"). The 0.8.x Tauri updater's key (keypair generated 2026-05-07):

- Public key: in the 0.8.x app's `tauri.conf.json` `plugins.updater.pubkey`; this repo keeps it as `crates/beebeeb-updater/keys/updater-0.8.pub` (safe to commit). Private key: `~/.tauri/beebeeb-desktop.key` on the build machine, mode 0600, **never commit**; generated without a password — before public release regenerate with a strong password and rotate the CI secret.
- CI secrets: `TAURI_SIGNING_PRIVATE_KEY` (key contents) + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (unset for the current passwordless setup). Inspect: `cat ~/.tauri/beebeeb-desktop.key.pub` (fingerprint `545D7BA77EDEA7E1` at generation). Lost key ⇒ already-installed clients need a manual reinstall.
- Rotation done (task 1240; checked 2026-10-29): the baked pubkey since 0.3.0 is `C6FADFD59D732197`. 0.3.0–0.8.3 were still signed with `545D7BA77EDEA7E1` (no 0.3.0+ install could update in-app); 0.8.4+ are signed with `C6FA`. The 0.8.x `release.yml` fails closed when a signature's key id differs from the baked one (`scripts/check-updater-signature-key.mjs`, in the build job and the manifest job). `updater_key_transition=true` only for a transition release, or to promote a tag cut before the guard after checking it by hand (`docs/RELEASING.md`).

## Storage location (task 1919, by COUNTRY since task 2050)

A storage location is a COUNTRY, never a city (owner, 2026-10-19; `docs/canon/data-residency.md` in the workspace). `europe` is the default; the rows are the API's (`GET /api/v1/me/region` `choices` = public `GET /api/v1/storage-regions`), rendered as sent — the desktop keeps NO list, city or country table; `account_set_region` sends the row's `id` verbatim with `regionStrict` stated. **No city is decoded at all** (`StorageChoice`/`RegionInfo` have no `city` field); the 0.8.x guard `tests/noCityInCopy.test.ts` (gone with the React frontend) failed on a city name or a `city` read anywhere in the React sources and the Tauri shell. Uploads send no region. Detail (older-server folding, "Only store in <country>", verify recipe): docs/runbook/desktop-task-history.md.

## Brand

- Icons: amber rounded square with lowercase `b` (`packaging/windows/icon.ico`, `packaging/macos/icon.icns`).
- App name "Beebeeb"; identifier `io.beebeeb.app`.

