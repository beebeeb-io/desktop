# This PC and Backups: the frame checklist (lane f-backups-history)

The Windows frames of the board's groups "This PC: Sync history and Available offline" (9) and
"Backups on Windows" (7 of the 9 items carry a Windows frame; DM-1408 and DM-1409 are Mac only).
"Mail Backup (proposal)" is out of this lane. 16 frames in all.

Built = drawn from the group's `Ui`, reachable by its controls, a scene by its frame id, gated in
light and dark. The gate lines per frame are in the PR body.

## This PC: Sync history and Available offline (`frames/this_pc/`)

| Frame | What | State |
| --- | --- | --- |
| DW-1301 | Sync history: every change of the last 30 days, grouped by day, the problem row with Try again | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-1302 | Sync history, filter This PC | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-1303 | Sync history, filter Other devices | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-0428 | Sync history, filter Problems: the Can't sync bar, Try again per item, Try all again | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-1304 | Sync history, "contract" in Search history | built; the caret is open (`search-caret` and the 3 boxes around it): a headless render has no focus |
| DW-1305 | Sync history on a PC that just signed in: No changes yet | built, 0 |
| DW-0715 | Available offline: what stays on this PC, sizes, Remove all from this PC | built, 0 |
| DW-1306 | Available offline: the row menu (Open, Show in File Explorer, Remove from this PC) | built; `status-details` open: the board's status line here is Files' ("4 on this PC …"), DW-0715's is the section's |
| DW-0120 | Available offline, nothing kept yet | built, 0 |

## Backups on Windows (a section of the settings view)

| Frame | What | State |
| --- | --- | --- |
| DW-0737 | Backups: the status card, This PC's folders with switches, other devices, services | not built |
| DW-1411 | Backups: the first backup of a folder running (612 of 1,820 files) | not built |
| DW-0738 | Add a folder: Windows' own folder picker, drawn for reference | not built |
| DW-1410 | Stop backing up Projects? keep the backup or move it to Trash | not built |
| DW-0739 | Another device's backup (Anna's iPad) | not built |
| DW-0742 | Delete this backup? with the authenticator code | not built |
| DW-0741 | Couldn't back up: three failures with Try again, one exclusion without | not built |

Built: 9 of 16 (all of This PC). Not built: the 7 Backups frames. They are a section of the
settings view; the settings shell (PR 292) reached `v2-native` after the This PC frames were done.
What exists for them: `frames/backups/source.rs`, the `BackupsSource` trait with the board's
content of DW-0737, DW-1411, DW-0739 and DW-0741 and the protected root (`is_protected`,
`BackupsRoot::allows`), with 3 tests and no view.

States the board gives a rule and no drawing, built from the rule's words (tests only): a filter or
a search that matches nothing draws an empty list (no invented copy); "Remove all from this PC"
emits `Intent::AskRemoveAll` and removes only after `Ui::remove_all_confirmed` (the ask has no
frame); the history's right-click menu (Show in File Explorer · Show in Beebeeb · Copy name) has
no frame and is not built.
