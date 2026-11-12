# Open files: the frame checklist (lane f-viewers)

The Windows frames of the board's groups "Open files: the viewers" (10), "Open files: states" (4) and "Text and
Markdown editing" (8): 22 frames, plus the Windows entry state of "Office: Docs, Sheets and Slides" (owner ruling
2026-11-11: office documents are not edited in the app in 0.9.0; a link that opens the web app).

Built = drawn from `viewers::Ui` (or, for DW-1520 to DW-1522, from `viewers::new_file` over the Files
window), reachable by its controls, a scene, gated in light and dark.
14 of 23 are built (lanes f-viewers and 2472, 2026-11-12): 7 at 0 failing in both themes, 7 with open
reds named below and in the PR bodies. The other 9 are not built; each line is the checklist and why.

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
| DW-1501 | A RAW photo: the camera's preview, the pill, the gallery strip, Share / Offline / Details / Trash | not built: no golden yet; a RAW preview decoder in Rune is not verified (task 2472 scope cut) |
| DW-1502 | The RAW's facts in the details panel | not built: as DW-1501 |
| DW-1505 | A PDF: page thumbnails, the page on grey, zoom, Find | not built: no golden yet; a PDF page renderer in Rune is not verified |
| DW-1506 | Find in a PDF: the find bar, matches highlighted (neutral) | not built: as DW-1505 |
| DW-1514 | A CSV or TSV as a table; a selected cell | not built: no golden yet (capture it first) |
| DW-1515 | An archive's list, without unpacking | not built: no golden yet (capture it first) |
| DW-1503 | The audio player with the folder as the queue | not built: no golden yet; audio playback is not in the app |
| DW-1504 | Playback in Windows' media controls (Windows draws Quick Settings) | not built: as DW-1503 |
| DW-1507 (entry) | An office document: the entry state with the link that opens the web app | not built: the entry state is an owner ruling (2026-11-11), not a board drawing, so there is no golden to gate it against |
