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
| DW-0738 | Add a folder: Windows' own folder picker, drawn for reference | not built: Windows draws it ("the file picker: Windows draws this"); Add a folder… asks the shell for it (`Intent::PickFolder`), which opens it on a worker (`shell::intents::pick_folder_for`) and hands the folder to the engine (slice A7) |
| DW-1410 | Stop backing up Projects? keep the backup or move it to Trash | built, 0; in a real build "Stop and move the backup to Trash" asks for the account password first (flagged below) |
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

**NOT ON THE BOARD (design deviation, flagged; lead ruling 2026-11-15 17:26, plan40 slice A7):** DW-1410's "Stop and
move the backup to Trash" asks for the ACCOUNT PASSWORD before anything moves, wherever the source proves it (the
engine: `BackupsSource::trash_needs_password`). The engine moves a backed-up folder to Trash only through its one
delete of a backup, which proves the password to Beebeeb (core's step-up, one single-use token); the board draws no
password field. The dialog's second step is drawn from the kit as "Delete this backup?" (DW-0742) draws its password:
- "Move the backup of <folder> to Trash?" · "The <size> backed up from <folder> will be moved to Trash. It leaves every
  device and is deleted for good when Trash empties. Nothing on this PC is touched." (without a size: "What is backed
  up from <folder> will be …") · the field "Account password" · Move to Trash (red, off until a password is typed;
  "Moving…" while it is on its way) · Cancel.
- A refused password stays in the step, under the field ("That password isn't right. Nothing was changed."); the folder
  is backed up as before. A move Beebeeb did not confirm closes the dialog and says so in the section. Esc, Cancel and a
  press outside change nothing.
- "Stop and keep the backup" asks for nothing (no password: nothing moves). The board's scene (the fixture) never opens
  the step, so compare.sh DW-1410 draws the board's first view, unchanged.
The board has no frame for the second step.

States the board gives a rule and no drawing, built from the rule's words (tests only): a filter or
a search that matches nothing draws an empty list (no invented copy); "Remove all from this PC"
asks once in the window's dialog card ("Remove all from this PC?", Remove all · Cancel; the ask has
no frame) and removes only on the yes; while the engine has no lists for a section it says why in
the section's empty state ("Reading this PC", "Couldn't read this PC", "Not available for this
account"; no frame), never "No changes yet"; on a real PC the status line says the window's own
sync word and the section's words, the board's sample counts only in the scenes; the history's right-click menu (Show in File Explorer · Show in Beebeeb · Copy name) has
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

## A parked change's way out (task 2530, wave 2 slice S3)

Sync history › Problems (DW-0428's row) draws, per problem, only the ways out the engine lists for it
(`drive_v2::status::SyncProblem::exits`; the source refuses any other):

| Row | Engine's ways out | The row's button | Reuses |
| --- | --- | --- | --- |
| a change that stopped after its tries, or waits out a backoff | Try again | Try again | DW-0428's Try again, as drawn |
| a change parked on a stale base (a viewer save another device overtook) | Keep both, Keep the other, Keep this PC's | Choose | DW-0428's row and button cell; the question below |
| a change whose earlier change was lost | Keep both, Keep the other | Choose | as above |
| a write the server refused (403) | Keep the other | Choose | as above |
| a change parked after a failed rekey | Try again | Try again | DW-0428's Try again |
| a change whose staged copy is gone (no bytes on this PC) | Remove from the list | Remove | DW-0428's row; the question below |
| a change that waits for an earlier one, a base not known, a sync-folder edit on a stale base, a held edit, files not checked yet | none | none | DW-0428's row, words only |

Words (the engine's, `drive_v2::status::parked_words`): "Not sent: this file changed in Beebeeb before this change
reached it. Choose which version to keep. Nothing was lost." · "Not sent: this account can no longer change this file
in Beebeeb. Your change stays on this PC. Nothing was lost." · "Not sent: Beebeeb lost track of the change made before
this one. Your change stays on this PC. Nothing was lost." · "Waiting: an earlier change to this file on this PC has
to be sent first. Nothing was lost." · "This PC no longer has the bytes of this change, so it can't be sent. The file
on Beebeeb is as it was."

**NOT ON THE BOARD (design deviation, flagged; lead ruling 2026-11-15 14:06):** the question a way out asks first
(`settle.rs`), drawn from the kit as the Files two-versions card (DW-0429) and the Remove-all ask are:
- Choose: "Two versions", the file's name, one line per answer offered, then the answers the row lists in the order
  Keep both (default) · Keep the other · Keep this PC's, and Cancel. Keep the other's line says where this PC's version
  goes: "Changes not sent, in Beebeeb's app data".
- Remove: "Remove this change?" · "This PC no longer has its bytes. The file on Beebeeb stays as it is." · Remove from
  the list · Cancel (no default answer: a stray Enter removes nothing).
- Cancel, Esc and a press outside change nothing. The question closes when its problem leaves the list or the section
  closes. What an answer did is said in the window's toast ("This PC's version is in Changes not sent, in Beebeeb's app
  data.", "This change moved since you saw it. Nothing was changed.", …).
The board has no frame for either card; the integrator's compare.sh covers DW-0428 and DW-1301, whose scenes hold no
parked row and draw as before.
