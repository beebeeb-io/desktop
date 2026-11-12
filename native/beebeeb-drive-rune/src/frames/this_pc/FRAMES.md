# This PC and Backups: the frame checklist (lane f-backups-history)

The Windows frames of the board's groups "This PC: Sync history and Available offline" (9) and
"Backups on Windows" (7 of the 9 items carry a Windows frame; DM-1408 and DM-1409 are Mac only).
"Mail Backup (proposal)" is out of this lane. 16 frames in all.

Built = drawn from the group's `Ui`, reachable by its controls, a scene by its frame id, gated in
light and dark. The gate lines per frame are in the PR body.

## This PC: Sync history and Available offline (`frames/this_pc/`)

| Frame | What | State |
| --- | --- | --- |
| DW-1301 | Sync history: every change of the last 30 days, grouped by day, the problem row with Try again | built; dark 0, light 1 pixel line open (`status-dot` edge 9 > 9) |
| DW-1302 | Sync history, filter This PC | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-1303 | Sync history, filter Other devices | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-0428 | Sync history, filter Problems: the Can't sync bar, Try again per item, Try all again | built; dark 0, light 1 pixel line open (`status-dot`) |
| DW-1304 | Sync history, "contract" in Search history | built; the caret is open (`search-caret` and the 3 boxes around it): a headless render has no focus |
| DW-1305 | Sync history on a PC that just signed in: No changes yet | built, 0 |
| DW-0715 | Available offline: what stays on this PC, sizes, Remove all from this PC | built, 0 |
| DW-1306 | Available offline: the row menu (Open, Show in File Explorer, Remove from this PC) | built, 0; under the open menu the status line reads Files' counts as this frame draws it ("4 on this PC …"), DW-0715's is the section's (PR body, Decision) |
| DW-0120 | Available offline, nothing kept yet | built, 0 |

## Backups on Windows (`frames/backups/`, a section of the settings view)

| Frame | What | State |
| --- | --- | --- |
| DW-0737 | Backups: the status card, This PC's folders with switches, other devices, services | built; light 0, dark 3 pixel lines open (the knobs of the three switches that are off, edge 11 > 9) |
| DW-1411 | Backups: the first backup of a folder running (612 of 1,820 files) | built, 0 |
| DW-0738 | Add a folder: Windows' own folder picker, drawn for reference | not built: Windows draws it ("the file picker: Windows draws this"); Add a folder… asks the shell for it (`Intent::PickFolder`) |
| DW-1410 | Stop backing up Projects? keep the backup or move it to Trash | built, 0 |
| DW-0739 | Another device's backup (Anna's iPad) | built, 0 |
| DW-0742 | Delete this backup? with the authenticator code | built; 1 pixel line open in each theme (`code-2`, the lit box's stroke on a half pixel, edge 32 > 9) |
| DW-0741 | Couldn't back up: three failures with Try again, one exclusion without | built, 0 |

Built: 15 of 16. Not built: DW-0738 (system UI). Gate, 30 runs (15 frames, light and dark):
21 with every line at 0, 9 with open lines, all named above.

States the board gives a rule and no drawing, built from the rule's words (tests only): a filter or
a search that matches nothing draws an empty list (no invented copy); "Remove all from this PC"
emits `Intent::AskRemoveAll` and removes only after `Ui::remove_all_confirmed` (the ask has no
frame); the history's right-click menu (Show in File Explorer · Show in Beebeeb · Copy name) has
no frame and is not built; a code the engine does not accept (DW-0742) empties the boxes and the
dialog stays, with no words (the board draws none); the password instead of a code ("or the
password without two-factor") has no frame and is not built; a folder the picker returns that
Beebeeb cannot back up changes nothing (no frame for the refusal).
