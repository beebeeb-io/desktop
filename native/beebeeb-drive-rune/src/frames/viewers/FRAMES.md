# Open files: the frame checklist (lane f-viewers)

The Windows frames of the board's groups "Open files: the viewers" (10), "Open files: states" (4) and "Text and
Markdown editing" (8): 22 frames, plus the text-document frames of "Office: Docs, Sheets and Slides" (see "The
office editor's frame" below; the 2026-11-11 note here, "office documents are not edited in the app in 0.9.0", was
replaced by the owner on 2026-11-12: the editor is in 0.9.0, contract `docs/superpowers/specs/2026-11-12-native-office-editor.md`).

Built = drawn from `viewers::Ui` (or, for DW-1520 to DW-1522, from `viewers::new_file` over the Files
window), reachable by its controls, a scene, gated in light and dark.
21 of 23 were built by those lanes (lanes f-viewers, 2472 and b19-viewers, 2026-11-12; the count is the two lanes' lists joined
at their merge): not at 0: see the integration ledger; the viewers still draw their own toolbar. The other 2
(DW-1504, DW-1507) were not built by those lanes; DW-1507 is built since (below). Each line is the checklist and why.

The Files toolbar under DW-1520 to DW-1522 has the Open in File Explorer button the board does not draw
yet (task 2445, B23): its 10 keys (`view-toggle`, `view-list`, `view-grid` and their icons, `sort-btn`,
`sort-icon`, `sort-caret`, `panel-toggle-btn`, `panel-icon`) fail geometry and pixels there, and so do
`toolbar` and `scrim` (the dialog's scrim covers the toolbar) on the frame tiles of that cluster only
(x 777 to 941, y 49 to 61). Those reds are the board's to clear, not this group's.

| Frame | What | State |
| --- | --- | --- |
| DW-1516 | A large file: "Decrypting on this PC", the size done, a bar, Cancel | built, 0 failing |
| DW-1517 | Too large to preview with the space this PC has: the numbers, Free up space…, Download to… | built, 0 failing |
| DW-1518 | Decrypting failed: nothing changed; Try again, Report a problem | built, 0 failing |
| DW-1512 | Code: colours, line numbers, Wrap / No wrap, Copy, Edit Ctrl+E | built; open reds: 4 pixel checks in light, 2 in dark (a glyph of the path under the control's edge, one line's tile); Wrap works |
| DW-1527 | A text file over 2 MB: read-only, the first 500 lines, Open in another app… | built, 0 failing |
| DW-1513 | Markdown rendered, Rendered / Source, Edit Ctrl+E, the details panel | built, 0 failing |
| DW-1519 | A newer version arrived while reading: a banner with Show it | built, 0 failing |
| DW-1523 | The editor: mono, line numbers, Markdown marks dimmed, the caret, Save Ctrl+S | built; open red: 1 pixel check (the unsaved dot's edge) in both themes |
| DW-1524 | Saved: the note with the new line, "Saved · encrypted · version 4" | built, 0 failing |
| DW-1525 | Leaving with changes: Save, Discard changes, Keep editing | built; open reds: the toolbar under the scrim (the board draws it without the dot and without Ctrl+S there, against DW-1523) and the Save button's edge |
| DW-1526 | Two versions while editing: Keep both, Keep this PC's, Keep Anna's iPhone's, Cancel | built; open reds: the toolbar under the scrim, as DW-1525 |
| DW-1520 | New's menu: New folder, New file…, New document…, Upload files…, Upload a folder… | built; only the Files toolbar's keys fail (above) |
| DW-1521 | New file: pick a type (Markdown, Text, Word, Excel, PowerPoint) | built; only the Files toolbar's keys fail (above) |
| DW-1522 | New text file: the name and the extension as its own field, suggestions | built; the toolbar's keys, and in dark 1 edge check: `field-0-caret` (its profile spans the selected name's last glyph and the selection's end, drawn by `ui/field.rs`: Rune ends the selection at the text's exact advance, the board's browser on the next whole pixel, and its light-on-dark glyph there is brighter) |
| DW-1501 | A RAW photo: the camera's preview, the pill, the gallery strip, Share / Offline / Details / Trash | built (lane b19-viewers); open reds: the toolbar's, and 4 pixel edges of `action-0..3` (see below) |
| DW-1502 | The RAW's facts in the details panel | built; open reds: the toolbar's |
| DW-1505 | A PDF: page thumbnails, the page on grey, zoom, Find | built; open reds: the toolbar's |
| DW-1506 | Find in a PDF: the find bar, matches highlighted (neutral) | built; open reds: the toolbar's; dark: 1 pixel edge of `find-caret`; Shift+Enter in the field (see below) |
| DW-1514 | A CSV or TSV as a table; a selected cell; sort by a column | built; open reds: the toolbar's |
| DW-1515 | An archive's list, without unpacking; a folder's row steps into it | built; open reds: the toolbar's |
| DW-1503 | The audio player with the folder as the queue | built; open reds: the toolbar's |
| DW-1504 | Playback in Windows' media controls (Windows draws Quick Settings) | built (`media/os_controls.rs`, D-MED-8; no artwork in 0.9.0) |
| DW-1507e | A text document in the editor's frame | built as a frame: see "The office editor's frame" |

## The photo, audio, PDF, table and archive viewers (lane b19-viewers, 2026-11-12)

- **The router by file type** is `kinds::viewer_of`; the one way a list's file opens here is `viewers::open_item`
  (it ends in `Event::OpenItem`). Files calls it for Enter and a double press on one file.
- **Pictures are the source's** (`ViewerSource::picture`: the photo, a strip thumbnail, the artwork, a PDF page), and
  the trait's default is none. Only the board's sample window has the fixture's pictures; any other source gets the
  plain tile (photo, artwork) or a blank page (PDF), never a sample.
- **The toolbar's reds:** the goldens of these seven frames carry the board's new toolbar button (36 pt), which the
  toolbar this group draws does not have yet. Until the board lane's toolbar lands here, the toolbar keys the rules
  name fail, and the file's own controls beside them (pill, zoom, Find, Share, Edit, more) sit 36 pt to the right;
  where the board's toolbar is full its path separators are narrower (DW-1506, DW-1515).
- **Shift+Enter in the find field** steps to the NEXT match, as Enter does: Rune's field submit carries no modifiers
  and the field's key hook is not asked for Enter (3 tries, in the PR body). Previous is the second chevron, and
  Shift+Enter while the keyboard is outside the field. A toolkit limit, named; the board wants Shift+Enter = previous.
- **This lane's own reds:** DW-1501 `action-0..3` (pixels, the bottom edge: Rune draws the white word on black 6 to 17
  of 255 brighter per row than Chromium at the same position); DW-1506 dark `find-caret` (pixel edge 10 > 9).
- **Controls whose target is not built yet** (Share, Move to Trash, Open in another app…, Edit in Sheets, Version
  history) say "Not built yet: <what>" in the viewer's toast and still record their intent for the shell.

## Open files on the engine (lane viewer-open, 2026-11-13)

- **The running app's source** is `engine_host::viewer::EngineViewerSource` (wired in `shell::state::init_on_engine`):
  the name and size from the local state, the content downloaded and decrypted in memory by the engine's own
  download path (or read from the copy kept on this PC). Shown: text, Markdown, CSV / TSV (as text), PNG / JPEG /
  GIF / WebP, and PDF (its pages rendered on this PC by a pure-Rust renderer, `engine_host::viewer_pdf`; the
  page and the thumbnail column scroll, only the thumbnails near the window are built; Find is not built on
  the engine's source: the text is not searched yet).
- **A list's Open asks the source first** (`ViewerSource::previews`, in `open_item`): a kind the source does not
  show here (on the engine: audio, archives, RAW and HEIC pictures) stays its own app's, as before. A route
  that names such a file anyway (Search's Open, a link) gets `Body::NotPreviewable`, and nothing is downloaded.
- **Two states are the engine's, not the board's** (no golden, drawn with the parts of DW-1516 to DW-1518):
  `Body::Stopped` (Beebeeb can't be reached, the session ended, the file was closed by a lock or an account
  change, the file is gone, nothing has synced on this PC yet, this PC's data could not be confirmed as this
  account's, the copy here could not be read) and `Body::NotPreviewable` (no viewer for this kind yet, over the
  preview's memory limit, content this app cannot read, a PDF it cannot read or that asks for a password), the
  latter with "Open in another app…" and "Save a copy…". A state says "decrypted" only for bytes that were
  downloaded and decrypted; for the copy on this PC it says "read from the copy on this PC".
- **A text read in part** is DW-1527's state: over 2 MB (the board's words), or under 2 MB with more lines or longer
  lines than the reader holds whole (`Cut::TooLong`, its own banner). Text and notes scroll.
- **"Open in another app…"** is the source's (`ViewerSource::open_elsewhere`): the system opens the file from the
  account's folder on this PC, so it needs sync on; each refusal has its sentence in the toast. The fixture has no
  way there and keeps "Not built yet".
- **Not bound on the engine, and said so on the press:** Edit and save (`ViewerSource::can_save`), Save a copy.
  Available offline is not read (`knows_offline`): the details panel draws no switch and the photo no action for it.
- **Leaving a file** overwrites the decrypted text it held (`OpenFile::wipe`, `Ui`'s drop); a `{:?}` of an open
  file prints kinds and counts, never a name or a word of it.

## The office editor's frame: a text document (lane gbb-d, 2026-11-14)

The FRAME of the editor, drawn from `ViewerSource` (`office.rs`, `office_over.rs`; the board's samples in
`office_board.rs`, the gate's keys in `scripts/keymap-office.js`). When this lane wrote it, no editing engine stood
behind it; one does now (office epic, 2026-11-16 to 18): `frames::office::edit` holds the live editor for Word, Sheets
and Slides on the desktop-office engines, and `engine_host::viewer_office` runs the helper, the save, the recovery blobs
and the lock snapshot. A save is asked of `ViewerSource::save_document`, a conflict's answer of `resolve_document`, the
command search of `commands`. The board's sample document (a window with no engine) still answers "not saved" and
"nothing found", and its other controls say "Not built yet". Gate at the lane's last run, light and dark: geometry 0 failing in 18 of 18 runs.

| Frame | What it is | State |
|---|---|---|
| DW-1507e | A text document: ribbon, outline, page, a selection with its tools, status line | built (live: `office/edit/word.rs`, `word_ribbon.rs`, `word_insert.rs`); open: light `pg-3`, `rib`, `rib-tools` (tile), dark `float` (edge) |
| DW-1511 | An older Word file (.doc): the banner, saved in its own format | built; open: light `rib`, `rib-tools` (tile) and 2 frame tiles; dark 0 |
| DW-1550 | The Insert tab, a table on the page | built; open: `crumb-dot` (edge) in both themes |
| DW-1551 | The command search (Ctrl+K) with "head" typed | built; open: light `rt-11`, `ft-0`, `rt-12`; dark `pal-caret`, `ol-4-row`, `ol-6-row`, `rt-11` (covered by the search) |
| DW-1552 | Changes that are not saved: the dot, Save on | built; open: `crumb-dot` both themes; light `rib`, `rib-tools` |
| DW-1553 | Saved: the next version, the toast | built; open: light `rib`, `rib-tools`; dark 0 |
| DW-1554 | Someone else saved first: four ways on | built; open: light `crumb-dot`; dark 0 |
| DW-1556 | The editor did not start | built; open: text `empty-text` both themes (the export records the paragraph's colour, the words inside are a shade darker and drawn so; pixels 0 on it), light `status-dot` (edge) |
| DW-1234 | Commands and keyboard shortcuts over the document | built (scene in `tray/`); open: `ol-close` both themes, dark `ol-4-row` to `ol-6-row` (covered by the dialog) |
| DW-1555 | "Preparing the editor" (a 257 MB download) | retired: the engines ship inside the app, nothing downloads (no scene) |
| DW-1557 | The licence notice (About, in the status bar) | built (`office/dialogs.rs`, scene `viewers::scene_licence`; the words say viewer and editor, and the MPL-2.0 media crates are listed in `THIRD-PARTY-NOTICES.md`) |
| DW-1508e, DW-1559 | A workbook in the editor; the Data tab (sort, filter, jump to a cell) | built (`office/edit/sheet_ribbon.rs`; grid state in `edit/sheet.rs`); gate numbers: see the vm-recording run |
| DW-1509e, DW-1560 | A deck in the editor; the slide's layout (the Design tab) | built (`office/edit/slide_ribbon.rs`, `edit/slide.rs`); gate numbers: see the vm-recording run |
| DW-1508, DW-1509, DW-1561 | Sheets, Slides (viewer) and Present | built (`office/sheets.rs`, `office/slides.rs`) |
| DW-1558, DW-1562 | New document from a template; Open With | DW-1558 built (`office/edit/new_document.rs`; the New menu's "New document…" and New file's Continue on an office type open it; Blank only, as contract Q7 recommended). DW-1562 built in the Files window (D-OV-14: `files/open_with_menu.rs`, scene `office/open_with.rs`), offered once Word's switch is on (`kinds::office_route`); the board's DW-1562 menu runs to Move to Trash like DW-0402 (R-OW2 (a), board fixed and recaptured at O-CP3) |

Why the open lines stay:
- `rib` / `rib-tools` (light), `pg-3`: the board's Italic "I" is a serif italic the app does not ship, and a text
  span's background has a fixed 3-point corner where the board's selection has 2.
- `crumb-dot`, `status-dot`, `pal-caret`: 7-point and 1.5-point marks at a fractional place after text; Rune puts
  them on the device pixel. Taking them off the grid measured worse (edge 69 against 15), so they stay on it.
- Keys covered by the search or the dialog (`ol-*`, `rt-*`, `ft-0`): what is compared at their place is what lies
  over them (the search's `esc` keycap at a fractional place, the dialog's title and rows), a fraction of a pixel off.
- DW-1511 light, 2 frame tiles: not diagnosed.
- DW-1556's status line says "Preparing the editor on this PC…" beside "We couldn't prepare the editor": the board's
  own words, kept as drawn; they belong to the held "preparing" flow and want the same ruling.

Not done here: typing in the document, Layout's and Review's tools (on no board frame), the unsaved-changes question
when a document with changes is left (there is no edit to lose until an editor exists), zoom, the About link.


## A newer version, on the engine (lane w4, 2026-11-15)

- **Design note, board vs decision 2505 (honesty trigger 2).** DW-1519's board draws "A newer version arrived from
  Anna's iPhone, 09:14." The engine does not know which device saved, and device names come after 0.9.0 (decision
  2505). On the engine the banner says **"A newer version arrived, 09:14."**: no device, the time this PC noticed
  it. The board's words stay as drawn (`Newer::from` set); an empty `from` drops the clause (`text::newer_banner`).
  The golden of DW-1519 is unchanged.
- **How it is noticed:** while a file is open, `viewer_change::check_for_newer` reads the local state's version
  (`Engine::edit_facts_for`) on a worker about every 1.5 s, one read at a time. A version past the one the view
  holds comes back as `Settled::Newer`; one banner per version. The view's own save moves what the view holds
  (`rebase`), so it never announces itself. An answer for another session or account is dropped.
- **Show it** opens the file again (`ViewerSource::newer` = `begin`): the version and content as Beebeeb has them
  now, arriving on the poll. With the editor open nothing is read again (the frame refuses).
- **Not traced:** that the sync engine raises `current_version` in the local state when another device's save lands
  while a file is open. The tests move it in the fake engine; whether it moves in a real sync is for the native rung.

## A newer version: fix round of #432 (lane fix432, 2026-11-15)

- **What a read found is weighed on the window's thread** (`viewer_change::take_settled` → `weigh`): the worker only
  reads (`Engine::edit_facts_for`) and sends the version, the opening it began under, and the time. `Opening::seen`
  is written by the window's thread alone and never goes down. A read for an earlier opening (before Show it, Try
  again or a reopen) is dropped. Reads run only for a text or a note (the kinds that draw the banner), and the
  window is woken for the next one (`ViewerSource::next_check`), so the banner comes without a press.
- **The view's own save is never "a newer version":** Saved moves the view to the version it became; a save
  stored here only ("Saved on this PC · sends when Beebeeb can be reached") keeps the view at the save's base, and
  the version it becomes once it lands is the engine's to tell (`EditFacts::own`, read without forgetting
  the queued save). A version past that one is another device's and is announced. The two-versions question does
  not move the view: after Cancel and Discard changes the next read still announces the newer version. A read that
  comes back in the same drain as a change's answer is weighed on the next pump. A save stored here keeps a newer
  version noticed while it waited (its banner shows once the editor closes).
- **Show it never labels older bytes with a newer version.** The label is read before the content (an open, Show
  it, Keep theirs). The engine does not read a copy on this PC whose version is behind the one it holds now, or is
  not known (`LocalCopy::Behind`): Beebeeb's version is downloaded, and the status line says **"The version in
  Beebeeb, decrypted on this PC · couldn't confirm the copy on this PC is this version"** (`Origin::BeebeebUnconfirmedHere`;
  delta check of #432: "not in sync" claimed more than is known for a copy of a version not known).
- **Why such a file does not open offline, though kept on this PC:** its copy here could not be confirmed as the
  version Beebeeb has, so it is never shown as that version, and the download is the only way to it; offline it says
  "Can't reach Beebeeb". Which copies: one older than the version pulled since (nothing replaces a kept file's bytes
  on that pull, Windows or Linux), one written by a save from this viewer (its landing records no agreed content:
  the bytes in the folder are the version before the save), one whose fetches served more than one version, and rows
  synced before agreed content was recorded. How many such rows an upgraded PC has is to be counted before 0.9.0.
- **Show it refused** (the file is gone, the session ended, it could not be read now): the version on screen stays
  open, its banner too, and the toast says why: **"Newer version not opened: the file is gone"** (also "the
  session ended", "nothing has synced on this PC yet", "this PC's data couldn't be confirmed as this account's",
  "Beebeeb is locked or signed out", "couldn't read it right now"). No board draws this toast: the words follow
  the status line's "Not opened: …".

## A newer version: fix round 2 of #432 (lane fix432, 2026-11-15)

- **"The view's own" belongs to the opening that saved.** The engine keeps, per file, the versions this session's
  viewer saves became (with the base each was saved from) and the saves still to land; a new opening (open, Try
  again, Show it) forgets them all (`Engine::forget_viewer_saves`). A version an earlier opening's save becomes is
  then announced as newer, and a save from the new opening's version is never continued from it (it answers the
  two-versions question).
- **A change's answer belongs to the opening that asked.** Show it and Try again wait while a change is on its way
  (`ViewerSource::saving`); Show it then says **"Newer version not opened yet: your change is still being saved"**.
  An answer for an earlier opening never replaces what is on screen: it is said (`Settled::Said`: "Your earlier
  edit was saved on this PC · sends when Beebeeb can be reached", "… saved · encrypted · version N", "… saved as a
  copy next to this file", "… not saved: another device saved this file meanwhile", "… not saved"), and its text is
  overwritten. No board draws these toasts. (Round 3 below replaces the record's forget with epochs and changes these words.)
- **A save stored here is labelled with the version it was saved from** (`Settled::Queued::version`, the base the
  engine used): the label is always the base of the text on screen.

## A newer version: fix round 3 of #432 (lane fix432, 2026-11-15)

- **Every opening has its own number in the engine (an epoch).** `begin` (open from Files, Try again, Show it) asks
  `Engine::viewer_opening`, which hands out a number never used before and starts an empty record of the file's
  viewer saves for it. Every read of the version and every save carries that number; the engine writes the record
  only for its own number and session, under one lock. A save of an earlier opening (or of an earlier source: a file
  opened again from Files is a new source) that ends late never makes, extends or changes a later opening's record:
  its landing is announced as a newer version, and a save from the new opening's version answers the two-versions
  question. *(Corrected, delta 3 of #432: this held only once the earlier save had landed. Before that, the new
  opening's save was queued after it and both landed unasked; see the next section.)* The records are the account's (`AccountRuntime::viewer_saves`): a lock, a sign-out or a sign-in empties
  them, a new opening drops other sessions', and each file keeps only its latest save still to land.
- **One change at a time, honestly.** Only the answer of the change in flight ends it; a press refused at once (the
  switch while a save is on its way) never does, so Show it still waits and no second save starts. A second answer
  that comes back in the same pump as another is kept for the next one (woken at once), never wiped unread.
- **The two-versions question names nobody while nobody is known.** The engine cannot tell another device from this
  PC's earlier opening, so the question says **"{file} · changed since you opened it"**, the other side is **"The
  other version"** and its button **"Keep the other version"** (the board's device name stays for a source that
  knows it). An earlier opening's conflict says "Your earlier edit was not saved: the file changed since you opened
  it". Show it during any change on its way says **"Newer version not opened yet: your last change is still on its
  way"**.
- **A copy here not settled with Beebeeb says so first:** "not in sync" wins over "couldn't confirm" when both hold.

## A save behind an unsent change: delta 3 of #432 (lane fix432, 2026-11-15)

- **A save of one opening is never queued after a change it is not based on.** While an earlier change to the file
  on this PC has not been sent yet, the engine refuses a new opening's save (`EditRefused::EarlierChangeNotSent`).
  That change can be an earlier opening's save, stored here or held back, or on macOS a save from another app.
  Before, the write queue put the new save after that change, and both landed one after the other without the
  two-versions question. The check runs inside the accept's own transaction (`ChainAfter`), so no write slips in
  between. A save of the SAME opening still follows its own unsent save: the text on screen is that save's text.
- **What the person sees:** **"Not saved: an earlier change to this file on this PC has not been sent yet. Your edit
  stays open."** (`Settled::Refused`). The editor keeps the edit, unsaved. Once that change lands, the banner offers
  its version, and a save from here asks the two-versions question. No board draws this toast.
- **The cost:** until that change is sent, the edit cannot be saved from this opening. Offline, or with the change
  held back, that can last a long time. The edit stays open but is kept nowhere else.
- **The way out: Save a copy (wave 2, task 2530 item 1).** The refusal is drawn as DW-1519's banner over the editor,
  its words wrapped, with one button, **Save a copy**: the two-versions question's Keep both, by its press path
  (`Ui::resolve(Keep::Both)` → `Engine::save_text_copy_for`). The edit becomes a new file next to the original, named
  as every conflict copy is; the original stays at its version with its queued change. After it, the window shows
  the copy, as Keep both does. A refusal of the copy is Keep both's "Not saved"; the edit stays. The banner goes with
  the edit and with the next press. Not the toolbar's "Save a copy…" of a file not shown here (`Intent::SaveCopy`,
  a decrypted copy to a place on this PC), which stays unbuilt.
- **Held back is said as held back.** When that earlier change is parked (its write used up its tries) or a held
  edit of task 2524, the engine answers `EditRefused::EarlierChangeHeld`, and the words promise no send.
- **Words: planner wave 2, not on the board** (no board draws this state):
  - not sent yet: **"Not saved: an earlier change to this file on this PC has not been sent yet. Your edit stays
    open: Save a copy keeps it as a new file."**
  - held back: **"Not saved: an earlier change to this file on this PC is held back and is not sent until it is
    settled in Sync history. Your edit stays open: Save a copy keeps it as a new file."**

## DW-0611, the video player (lane mb2-surface, 2026-11-15)

- **D-MED-4:** Rune draws the player controls in the board's geometry; the board says Windows' own player.

## Wired since (waves 16 to 18, 2026-11)

The open file's own controls act: Share, Version history, Move to Trash, Report a problem, Free up space, Keep on this PC,
Open in Files, the links and New (w17-viewer-actions); Save a copy and Download to, with progress
(w17-viewer-save, `shell/viewer_save.rs`; the Save dialog is `Platform::pick_save_path`). Edit on a file the editor does not take says why in a toast
(`why_not_edit`). DW-1504 is `media/os_controls.rs`.
Edit in Sheets and Open in another app are still not taken by the shell.
