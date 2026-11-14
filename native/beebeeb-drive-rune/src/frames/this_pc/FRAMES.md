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
| DW-0740 | Mail Backup (proposal): on, its status, accounts, Turn off | built, 0 |
| DW-0740b | Mail Backup is off: the kept folder, Turn on… | built, 0 |
| DW-1401 | Turn on Mail Backup: what it does, before anything is asked | built, 0 |
| DW-1402 | Add a mail account: address, IMAP server, port, app password | built; light 0, dark 1 pixel line open (`field-3`, the focused field's ring on the dialog's quarter point, edge 12 > 9) |
| DW-1403 | Add a mail account: the server refused the password, nothing was saved | built, 0 |
| DW-1404 | Choose folders of the mailbox that signed in | built, 0 |
| DW-1405 | The first archive runs: 6,912 of 17,480 messages, Pause | built, 0 |
| DW-1406 | Couldn't back up: the app password was changed or revoked; Update password | built, 0 |
| DW-0743 | Turn off Mail Backup? with the authenticator code | built; light 0, dark 2 pixel lines open (`list-1-body`, `item-1-0` under the dialog's shadow, edge 9 > 9) |
| DW-0743b | Keep Mail Backup's files? Keep them, or Move to Trash | built, 0 |

Built: 25 of 26. Not built: DW-0738 (system UI). Gate, 50 runs (25 frames, light and dark):
39 with every line at 0, 11 with open lines, all named above.

States the board gives a rule and no drawing, built from the rule's words (tests only): a filter or
a search that matches nothing draws an empty list (no invented copy); "Remove all from this PC"
emits `Intent::AskRemoveAll` and removes only after `Ui::remove_all_confirmed` (the ask has no
frame); the history's right-click menu (Show in File Explorer · Show in Beebeeb · Copy name) has
no frame and is not built; a code the engine does not accept (DW-0742) empties the boxes and the
dialog stays, with no words (the board draws none); the password instead of a code ("or the
password without two-factor") has no frame and is not built; a folder the picker returns that
Beebeeb cannot back up changes nothing (no frame for the refusal).

Mail Backup (lane gbb-d, 2026-11-14) is the board's proposal, drawn from `BackupsSource`'s mail methods, whose
defaults answer "nothing" (no mail engine exists): only a source that answers `mail()` gets the page, so a build
that ships keeps asking the shell (`Intent::MailBackup`) and draws no sample values. States built from a rule's
words with no drawing (tests only): Connect and Start backup are off (half strength) until they can act; the
account step while the server has not answered says "Connecting…"; the first archive paused says "Mail backup is
paused" with Resume. Not built: the other refusals of DW-1403 (server not found, port closed, TLS refused: the
board gives no sentences), filling the server from the address's domain (DW-1402's rule: it needs a lookup), and
an account row's own page (the board draws its chevron and no frame for it: the row is drawn and takes no press).
