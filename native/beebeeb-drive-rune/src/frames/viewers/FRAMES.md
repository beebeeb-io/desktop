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
| DW-1504 | Playback in Windows' media controls (Windows draws Quick Settings) | not built |
| DW-1507 | A text document in the editor's frame | built as a frame: see "The office editor's frame" |

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
`office_board.rs`, the gate's keys in `scripts/keymap-office.js`). No editing engine stands behind it: a save is asked
of `ViewerSource::save_document`, a conflict's answer of `resolve_document`, the command search of `commands`; the
trait's defaults answer "not saved" and "nothing found", and the frame draws exactly that. Every other control says
"Not built yet". Gate at the lane's last run, light and dark: geometry 0 failing in 18 of 18 runs.

| Frame | What it is | State |
|---|---|---|
| DW-1507 | A text document: ribbon, outline, page, a selection with its tools, status line | built; open: light `pg-3`, `rib`, `rib-tools` (tile), dark `float` (edge) |
| DW-1511 | An older Word file (.doc): the banner, saved in its own format | built; open: light `rib`, `rib-tools` (tile) and 2 frame tiles; dark 0 |
| DW-1550 | The Insert tab, a table on the page | built; open: `crumb-dot` (edge) in both themes |
| DW-1551 | The command search (Ctrl+K) with "head" typed | built; open: light `rt-11`, `ft-0`, `rt-12`; dark `pal-caret`, `ol-4-row`, `ol-6-row`, `rt-11` (covered by the search) |
| DW-1552 | Changes that are not saved: the dot, Save on | built; open: `crumb-dot` both themes; light `rib`, `rib-tools` |
| DW-1553 | Saved: the next version, the toast | built; open: light `rib`, `rib-tools`; dark 0 |
| DW-1554 | Someone else saved first: four ways on | built; open: light `crumb-dot`; dark 0 |
| DW-1556 | The editor did not start | built; open: text `empty-text` both themes (the export records the paragraph's colour, the words inside are a shade darker and drawn so; pixels 0 on it), light `status-dot` (edge) |
| DW-1234 | Commands and keyboard shortcuts over the document | built (scene in `tray/`); open: `ol-close` both themes, dark `ol-4-row` to `ol-6-row` (covered by the dialog) |
| DW-1555, DW-1557 | "Preparing the editor" (a 257 MB download) and the licence notice of a bundled office suite | NOT built, held for a ruling: the board draws a downloaded third-party editor, the merged contract builds on in-house engines with nothing to download |
| DW-1508, DW-1509, DW-1559, DW-1560, DW-1561 | Sheets and Slides | NOT built |
| DW-1558, DW-1562 | New document from a template; Open With | NOT built (DW-1558's four templates differ from the contract's "Blank only"; DW-1562's menu is shorter than DW-0402's file menu: both need a ruling) |

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

