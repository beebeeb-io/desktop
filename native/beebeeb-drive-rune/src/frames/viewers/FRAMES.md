# Open files: the frame checklist (lane f-viewers)

The Windows frames of the board's groups "Open files: the viewers" (10), "Open files: states" (4) and "Text and
Markdown editing" (8): 22 frames, plus the Windows entry state of "Office: Docs, Sheets and Slides" (owner ruling
2026-11-11: office documents are not edited in the app in 0.9.0; a link that opens the web app).

Built = drawn from `viewers::Ui`, reachable by its controls, a scene, gated in light and dark.
11 of 23 are built (lane f-viewers, 2026-11-12): 7 at 0 failing in both themes, 4 with open reds
named below and in the PR body. The other 12 are not built; each line is the checklist.
Lane b19-viewers built 7 more (2026-11-12): 18 of 23 are built, 5 are not (DW-1520 to DW-1522, DW-1504, DW-1507).

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
| DW-1520 | New's menu: New folder, New file…, New document…, Upload files…, Upload a folder… | not built |
| DW-1521 | New file: pick a type (Markdown, Text, Word, Excel, PowerPoint) | not built |
| DW-1522 | New text file: the name and the extension as its own field, suggestions | not built |
| DW-1501 | A RAW photo: the camera's preview, the pill, the gallery strip, Share / Offline / Details / Trash | built (lane b19-viewers); open reds: the toolbar's, and 4 pixel edges of `action-0..3` (see below) |
| DW-1502 | The RAW's facts in the details panel | built; open reds: the toolbar's |
| DW-1505 | A PDF: page thumbnails, the page on grey, zoom, Find | built; open reds: the toolbar's |
| DW-1506 | Find in a PDF: the find bar, matches highlighted (neutral) | built; open reds: the toolbar's; dark: 1 pixel edge of `find-caret`; Shift+Enter in the field (see below) |
| DW-1514 | A CSV or TSV as a table; a selected cell; sort by a column | built; open reds: the toolbar's |
| DW-1515 | An archive's list, without unpacking; a folder's row steps into it | built; open reds: the toolbar's |
| DW-1503 | The audio player with the folder as the queue | built; open reds: the toolbar's |
| DW-1504 | Playback in Windows' media controls (Windows draws Quick Settings) | not built |
| DW-1507 (entry) | An office document: the entry state with the link that opens the web app | not built |

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
