# Photos: the frame checklist (lane f-photos)

The Windows frames of the board's group "Photos" (FRAMES.md "Photos (13)"; `frames.json` section `Photos`):
13 frames, all in the main window (1278 x 798 client area).

Built = drawn from `photos::Ui` and `PhotosSource`, reachable by its controls, a scene in `shell/mod.rs`
(`--scene DW-0601` …), tested. Gated = `scripts/compare.sh <id>` ends with six counts of 0, light and dark.
The six counts per frame, and why a red one is red, are in the PR body.

| Frame | What | Built | Gated |
| --- | --- | --- | --- |
| DW-0601 | The timeline: toolbar, source pills, the devices' backup status, days, 7 across | yes | light and dark |
| DW-0116 | Empty: "No photos yet" (also when a device filter has nothing) | yes | light and dark |
| DW-0603 | The Photos across menu: 3, larger · 4 · 5 · 7, smallest | yes | light and dark |
| DW-0602 | Selecting: three checked, the selection bar (Ctrl+click starts it) | yes | dark; light: 3 pixel keys red (the checked circle's top row) |
| DW-0604 | The Photo sources dialog: device switches, folders from Files | yes | dark; light: 1 pixel key red (one switch, edge 9.0 > 9) |
| DW-0605 | One photo open: top bar, the photo, the strip, Share · Offline · Details · Trash | yes | no: the four action labels (white text), the title, the arrows |
| DW-0605b | The original is downloading: "12.4 of 30.9 MB", the thin bar | yes | no: as DW-0605 |
| DW-0606 | The ⋯ menu: Save to PC… · Move… · Rename · Duplicate | yes | no: as DW-0605 |
| DW-0607 | Share a photo: the dialog | yes | dark; light: 3 pixel keys red (the action labels behind the scrim) |
| DW-0608 | Offline: "On this PC", the message | yes | no: as DW-0605 |
| DW-0609 | Details: the dialog | yes | dark; light: 3 pixel keys red (the action labels behind the scrim) |
| DW-0610 | Trashed: the next photo, "Moved to Trash" with Undo | yes | no: as DW-0605, and "Undo Ctrl+Z" (1 text, 1 pixel key) |
| DW-0611 | A video playing: the player's bar | yes, without the board's note to the reader | no |

The brief also names states the board does NOT draw for this group. No frame id exists for them, so there is
no golden to gate against:

- a thumbnail that is not there yet: built (the neutral tile; `PhotosSource::thumbnail` returns `None`), tested, not gated;
- albums, loading placeholders, the offline state, the error state, a comparison tool: not built, the board draws none.
