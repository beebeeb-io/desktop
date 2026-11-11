# beebeeb-engine

The running sync engine of the desktop app, without a UI toolkit: the state database, the API client, the
credential stores, the runner and its tick, Windows Cloud Files, and the view models a window lists from.
`src-tauri` (the Tauri shell) and `native/beebeeb-drive-rune` (the native shell) both build on it. Nothing here
names Tauri (`scripts/check-engine-boundary.sh`).

Tests: `cargo test --locked` in this directory. The `test-support` feature is for the tests of other crates only;
a release build with it does not compile.

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
