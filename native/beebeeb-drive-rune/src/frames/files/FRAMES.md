# Files frames: the lanes' checklist (f-files-actions, then f-files-rest)

Windows frames of the FRAMES.md groups "Files: the full Drive in the window", "Browse files" and
"Folders, Backups and uploads", plus the list's states drawn under "Global elements on Windows".
Status is `built` (gated light and dark, both truth lines at 0 failing) or `not built`.
State after the review delta (2026-11-11): 11 built here (plus DW-0401, built before), 38 not built.
State after lane f-files-rest (2026-11-12, PR 305): 16 built (2 of them with a gate not at 0), 34 not built here.
Of those 34, the six list states DW-0101, 0102, 0114, 0118, 0119 and 0121 are built by the global lane (PR 299): 28 are built nowhere.
State after lane files-browse, PR 1 (2026-11-12): DW-0422, 0423, 0424, 0425 and 1110 built, none at 0 (the reds are named in their rows).
Not on the board, added on the owner's request (PR 305): Free up space (file menu, details panel) and the
toolbar's Open in File Explorer; until the board draws them and the goldens are recaptured, NO Files frame
gates at 0: the toolbar button moves ten keys in each of the 16 frames (`view-toggle`, `view-list`,
`view-grid` and their icons, `sort-btn`, `sort-icon`, `sort-caret`, `panel-toggle-btn`, `panel-icon`), and
Free up space moves more in DW-0404, DW-0406 and DW-1104. Where a row below says "0 failing on all six", read:
before those two additions (at `1b2da021`). The 32 truth lines at the PR's head are in the PR body.

| Frame | Group | What it is | Status |
| --- | --- | --- | --- |
| `DW-0401` | Files | The Windows app's main view: the whole account's files, from the copy on this PC, with what is on this PC at a glance. | built (design-system lane, PR 286) |
| `DW-0402` | Files | Every action on a file, where a Windows user looks for it. | built (light and dark: 0 failing on all six) |
| `DW-0403` | Files | Act on several files at once. | not built: golden captured; needs the selection bar and the two stacked pages (one rotated: the gate reads a layout box, not the turned one) |
| `DW-0404` | Files | Choose the order of the list, the same choices as the iPhone. | built (light and dark: 0 failing on all six) |
| `DW-0405` | Files | Trash without a question, because Undo and Trash both bring it back. | built (light and dark: 0 failing on all six) |
| `DW-0406` | Files | Duplicate makes the copy at once, next to the original. | built (light and dark: 0 failing on all six) |
| `DW-0407` | Files | Share a file with a link, or invite someone. | not built as a frame (the dialog is the Shared lane's); the entry point works: Share… emits `Intent::Frame("DW-0407")` |
| `DW-0408` | Files | Look at a file without leaving Beebeeb: Preview in the window; Open hands it to its app. | not built; Space and Preview emit `Intent::Frame("DW-0408")` |
| `DW-0408b` | Files | Preview on a file Windows cannot preview: say so, instead of a blank panel. | not built |
| `DW-0409` | Files | Move the selection to another folder. | built (lane f-files-rest, PR 305: light and dark at 0 failing on all six before the toolbar button). Move… opens it; Tab goes round it, and Left goes back up from a folder without folders; `FilesSource::move_to`. Not built: dragging a row onto a folder or a sidebar item |
| `DW-0410` | Files | Rename where the name is, as File Explorer does. | built, gate NOT at 0 (PR 305): F2 and Rename work in the row through `FilesSource::rename`; the board's caret takes 3.5 pt of the line, a text field's takes none, so `.pdf` stands 3.5 pt further left (geometry 1, pixels 5) |
| `DW-0410b` | Files | A name that is already used in the folder: say so and keep the old name. | built, gate NOT at 0 (PR 305): the error style and the tip are drawn and work; same caret cause as DW-0410, and the board's status line says "4 on this PC" with five such rows (text 1) |
| `DW-0411` | Files | Everything about one file, the same rows as the iPhone's Details. | not built; Alt+Enter and Details emit `Intent::Frame("DW-0411")` |
| `DW-0412` | Files | Keep a file on this PC, also without a connection. | not built: golden captured; the action works (state, toast "Keeping on this PC"), the row's progress bar and "Syncing" are not drawn |
| `DW-0413` | Files | Go back to an earlier version without losing the current one. | not built; Version history… emits `Intent::Frame("DW-0413")` |
| `DW-0414` | Files | Everything that adds to the folder you're in: the one amber control. | built (light and dark: 0 failing on all six) |
| `DW-0415` | Files | Pick files from this PC to upload to the folder you're in. | not built; Ctrl+O and Upload files… emit `Intent::Frame("DW-0415")` |
| `DW-0416` | Files | Pick photos to upload: the file picker's Photos section, which needs no Photos access. | not built |
| `DW-0417` | Files | Make a folder in the folder you're in. | built (PR 305: light at 0 failing on all six; dark pixels 2: the caret's rounded rim, and row-8's edge under the dialog's title). Ctrl+Shift+N and New folder open it; `FilesSource::new_folder`, then the folder is shown (DW-1106) |
| `DW-0480` | Files | Upload a whole folder, with everything in it, keeping its structure. | not built; Upload a folder… emits `Intent::Frame("DW-0480")` |
| `DW-0481` | Files | Drag files from File Explorer into the list to upload them. | not built as a frame (drop highlight, drag badge); a drop works: `Intent::Upload(folder, files)` |
| `DW-0420` | Folders | The same view one level deeper. | not built: golden captured; the fixture lists the folder's 7 rows and the path title is drawn; the four photo thumbnails and the video mark are not |
| `DW-0420b` | Folders | Long content: a folder of 212 items scrolled two-thirds down, and names longer than a row. | built (lane files-browse, `long_list.rs`): 212 items in one lazy list; names over 58 characters shortened in the middle with the whole name under the pointer; letters jump to a name. The frame is drawn from row 139 by a scene value (the gate draws one frame and cannot scroll). Gate not at 0: the board lists 0140 before 0141 (by number), the app sorts by name, so rows 1 and 2 are swapped; the overlay scrollbar is not drawn; the details toggle as in DW-0422 |
| `DW-0421` | Folders | The grid: thumbnails first, the way the iPhone's grid shows them. | built (`long_list.rs`, on the global lane's grid): the view switch in Lisbon trip; thumbnails from the fixture's `ThumbSource`, the video's length. Gate not at 0: only the details toggle the board draws pressed (`panel-toggle-btn`, `panel-icon`) |
| `DW-0422` | Folders | Every device's backup and every Beebeeb service, in one protected folder. | built (lane files-browse, `protected.rs`): the sidebar's Backups (Ctrl+4), the Backups row in Files and Back lead here; one row selected at a time. Gate not at 0: `item-count` says "5 items" (board "5 folders": the toolbar's title is the one-toolbar lane's), and the board draws the details toggle pressed with no panel (`panel-toggle-btn`, `panel-icon`) |
| `DW-0423` | Folders | What one device backed up: this PC's folders, contacts and calendar. | built (`protected.rs`): Enter or a double-click on Anna-Laptop; read-only, Delete is Delete this backup (DW-0742). Gate not at 0: as DW-0422, and `status-details` (the board repeats DW-0401's sentence; the app counts the 5 rows) |
| `DW-0424` | Folders | A folder a Beebeeb service manages: open and download only. | built (`protected.rs`): the note with Settings; the service's order is kept; New dimmed, drops refused. Gate not at 0: as DW-0423 |
| `DW-0425` | Folders | Say why a protected folder can't change, in the core's words. | built (`protected.rs`): the message, in the source's words, on every way to rename, move or delete there (menu, Delete, F2, Ctrl+D, Move, several selected). Gate not at 0: as DW-0423 |
| `DW-0426` | Folders | Uploads in progress, in their folder and in the status line. | not built |
| `DW-0427` | Folders | Every upload with its state: what needs attention, what is moving, what waits. | not built |
| `DW-0429` | Folders | A file changed in two places at once: nothing is overwritten until Anna picks. | not built |
| `DW-1101` | Browse | The first screen of the window: folders first, then files; New is the one amber thing. | built (light and dark: 0 failing on all six) |
| `DW-1102` | Browse | A folder remembers its own view: here, a grid with the details panel on a photo. | not built |
| `DW-1103` | Browse | View and sort from a right-click on the list's background. | not built |
| `DW-1104` | Browse | Right-click is the PC's press and hold: the file menu on Harbour.heic. | built (light and dark: 0 failing on all six) |
| `DW-1105` | Browse | Instant search on the list already on this PC, and a plain note of what it can't do. | not built |
| `DW-1106` | Browse | A folder with nothing in it yet. | built (light and dark: 0 failing on all six) |
| `DW-1107` | Browse | The first sign-in on a PC: the list downloads and is decrypted here, and Beebeeb can be used before it's finished. | not built |
| `DW-1108` | Browse | Offline, everything stays visible; only what isn't on this PC is dimmed and says why. | not built |
| `DW-1109` | Browse | The session ended after 90Â days without use: the list and the key are still here, so only the password is needed. | not built |
| `DW-1110` | Browse | Trying to delete Mail Backup: the protected folder message. | built (`protected.rs`): Delete or F2 on Mail Backup in Backups: the message, in the source's words for the folder itself. Gate not at 0: as DW-0423 |
| `DW-0101` | List states | No connection: the list stays and says how old it is. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
| `DW-0102` | List states | Some items can't sync: say how many, and that the rest is fine. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
| `DW-0107` | List states | A short confirmation that needs nothing. | built (light and dark: 0 failing on all six) |
| `DW-0108` | List states | A message with Undo stays a little longer. | built (light and dark: 0 failing on all six) |
| `DW-0109` | List states | Something failed: say what, and how to fix it. | built (light and dark: 0 failing on all six): the message of a failed action, with Try again |
| `DW-0114` | List states | A new account has no files yet: one way forward. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
| `DW-0118` | List states | After the first sign-in the list loads in the background; the window works at once. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
| `DW-0119` | List states | Thumbnails arrive one by one: plain tiles until then. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
| `DW-0121` | List states | Offline, and the file isn't on this PC: say so instead of opening nothing. | built by the global lane (`frames::global`, PR 299; its checklist is `../global/FRAMES.md`) |
