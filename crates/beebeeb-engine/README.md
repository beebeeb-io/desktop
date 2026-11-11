# beebeeb-engine

The running sync engine of the desktop app, without a UI toolkit: the state database, the API client, the
credential stores, the runner and its tick, Windows Cloud Files, and the view models a window lists from.
`src-tauri` (the Tauri shell) and `native/beebeeb-drive-rune` (the native shell) both build on it. Nothing here
names Tauri (`scripts/check-engine-boundary.sh`).

Tests: `cargo test --locked` in this directory. The `test-support` feature is for the tests of other crates only;
a release build with it does not compile.

## For a host: one engine handle (`app::Engine`)

```rust
use beebeeb_engine::app::{Engine, EngineConfig, PlatformCredentials, SyncStart};
use beebeeb_engine::host::ChannelHost;

let (host, messages) = ChannelHost::new(|| { /* post a wake-up to the UI loop; nothing else */ });
let engine = std::sync::Arc::new(Engine::open(
    EngineConfig { account_id, state_dir, api_base: beebeeb_engine::runner::api_base_url() },
    host,
    PlatformCredentials,
)?);
match engine.restore() { /* GateState: SignedOut, NeedsRecoveryPhrase, Locked, Ready, KeyStoreUnavailable */ }
engine.sign_in_password(email, password)?;   // -> NextAfterSignIn::{TwoFactor, RecoveryPhrase, Drive}
engine.two_factor(code)?;                    // after TwoFactor
engine.unlock_with_phrase(phrase)?;          // after RecoveryPhrase
engine.start_sync(sync_root)?;               // SyncStart::{Started, AlreadyRunning, NoSession}
engine.listing(None, Sort::default(), 0, 200)?; // the Files rows: Backups root first, each row marked
engine.pause(); engine.resume(); engine.stop(); engine.lock(); engine.sign_out_keeping_local_data()?;
```

**Threading.**

| call | blocks? | runs on |
| --- | --- | --- |
| `open` | no network; starts the handle's own tokio runtime (threads named `beebeeb-engine`) | the caller |
| `restore`, `gate` | credential store only (DPAPI or the Keychain can take a moment) | the caller |
| `sign_in_password`, `two_factor`, `unlock_with_phrase`, `sign_out_keeping_local_data` | yes: the network (30 s per request at most) and the password's key derivation | the caller, waiting on the runtime |
| `start_sync`, `stop`, `lock` | until the engine exists, or its task has ended | the caller, waiting on the runtime |
| `pause`, `resume`, `is_paused` | no | the caller |
| `listing`, `state_db` | a local database read | the caller |
| the sync engine itself | never returns to you | the runtime's threads |

Every method is synchronous and takes `&self`: call them from a worker thread, never from the UI thread. One
session step runs at a time (a second call waits for the first). The UI thread learns about the engine through
the `EngineHost` given to `open`: a `ChannelHost` queues each `HostMessage` (typed `EngineEvent`s, notices, "open
the conflict window") and calls the wake hook on the ENGINE's thread; the UI thread drains the receiver when
woken. `{:?}` of a message prints its kind and never a name or a path.

**Rules the handle keeps** (from `signin/`, the same ones the Tauri shell uses): a stored vault key is loaded
only when it is provably this account's (`signin::store::load_session_from_vault`, task 2447); a sign-in removes
a key that is not this account's before it stores its token; the phrase's key is stored only after the server
confirmed it; an engine starts only with the keys of the session in memory, after the local data is bound to
that account. A start onto another account's local data is refused on every OS and deletes nothing.

**Not in the handle yet:** the plan check (`GateState::NoPlan` is never answered), device approval, passkeys,
Windows Hello, the legacy flat credential layout (read by the Tauri shell only), a sign-out that sends or deletes
local data (only "keep"), a startup probe of the stored token against the server, storage numbers. The API base,
the config dir and the state dir are process-wide in the engine (`BB_API_BASE`, `state_paths`), and the
credential store's service name and the config folder name are constants: a second app identity on the same
user account needs the app-identity port (PR 288) before it may open a handle with `PlatformCredentials`.

Contract test: `tests/host_contract.rs` (the local fake API with a real OPAQUE server side, the in-memory
credential store).

## For frame lanes: the Backups folder (LD-0113)

The window is an app surface, so it follows the web app's rule, not Explorer's.

**Listing** (`drive_v2::model::children_of`, which `EngineSource::page` calls):

- every `DriveItem` carries `backup: BackupMark { position, is_protected }`. `position` is core's
  `BackupPosition` (`Root`, `DeviceFolder`, `Manifest`, `CategoryFolder`, `Inside`, `Outside`);
  `is_protected` is true for the Backups root only: the row with the lock and the "Read-only" chip
  (`beebeeb_core::backup::MSG_BADGE`);
- a device's manifest (`.device.json`) is not listed and not counted;
- the Backups root is the first row in every sort and both directions (`model::backups_root_first`).

**Before any action on a row**, ask `backup_actions`:

```rust
use beebeeb_engine::backup_actions::{self, Action, Verdict};
use beebeeb_engine::backup_protect;

let own = backup_protect::own_backups(); // what this device backs up now
match backup_actions::check(&db, &own, &item.id, Action::Rename)? {
    None => { /* the row is gone */ }
    Some(Verdict::Allowed) => { /* do it */ }
    Some(Verdict::Refused(words)) => { /* show `words`; never log them (they can carry a local path) */ }
    Some(Verdict::UseDeleteBackup) => { /* Trash / DeleteForever on a backup: offer Action::DeleteBackup */ }
    Some(Verdict::NeedsPassword) => { /* Action::DeleteBackup: ask the password first */ }
    Some(Verdict::NotABackup) => { /* Action::DeleteBackup on a row that is no whole backup: not offered */ }
}
let menu = backup_actions::allowed_actions(&db, &own, &item.id)?; // every action that is not refused
```

`Err` means the database could not be read: refuse the action, never treat it as allowed.

| Action | the row is | at or under the Backups root |
| --- | --- | --- |
| `Trash`, `DeleteForever` | the item | root: refused (`MSG_DELETE`). A device's folder or one backed-up folder: `UseDeleteBackup`. Inside a backup: allowed. |
| `Rename`, `Move` | the item | refused: root `MSG_RENAME` / `MSG_MOVE`, else `MSG_READ_ONLY` |
| `MoveInto`, `UploadInto`, `NewFolderIn` | the DESTINATION folder | refused (`MSG_READ_ONLY`). For a move, ask `Move` of the item and `MoveInto` of the destination. |
| `Edit` | the file | refused (`MSG_READ_ONLY`) |
| `Share` | the item | a folder: refused (`backup_actions::MSG_NO_FOLDER_SHARE`). A single file: allowed. |
| `FreeUpSpace` | the item | allowed (nothing changes in the cloud) |
| `DeleteBackup` | a device's folder, or one backed-up folder | `NeedsPassword` (dialog text: `MSG_DELETE_BACKUP_STEP_UP`) |

A row that is this device's own backup of a folder it backs up now answers "This is a backup of `<path>`. Change
the original instead." for every change (core's `own_backup_refusal`); the path is shown, never logged.

**Delete a backup** (`backup_delete::delete_backup`, async): for a row whose `DeleteBackup` verdict is
`NeedsPassword`, pass the password the person typed. It asks the rule again, proves the password to the server
(core's OPAQUE step-up), and spends the single-use confirmation token on the delete of that one folder
(`DELETE /api/v1/files/{id}`, a move to Trash, as the web app does). `DeleteBackupError` says why nothing was
deleted: `WrongPassword`, `Refused(words)` (the root, this device's own active backup), `NotABackup`, `NotFound`,
`SignedOut`, `RateLimited(seconds)`, `Offline`, `Failed(code)`. The rows leave the listing on the engine's next pull.

Not built yet: the engine entry points for "Free up space", "Keep on this PC", "Open/Show in File Explorer" and
opening a file for the in-app viewers.
