# Photos: the frame checklist (lane f-photos)

The Windows frames of the board's group "Photos" (FRAMES.md "Photos (13)"; `frames.json` section `Photos`):
13 frames, all in the main window (1278 x 798 client area).

Built = drawn from `photos::Ui` and `PhotosSource`, reachable by its controls, a scene in `shell/mod.rs`,
gated in light and dark. The gate's six counts per frame are in the PR body.

| Frame | What | State |
| --- | --- | --- |
| DW-0601 | The timeline: toolbar, source pills, the devices' backup status, days, 7 across | not built |
| DW-0116 | Empty: "No photos yet" (also when a device filter has nothing) | not built |
| DW-0603 | The Photos across menu: 3, larger · 4 · 5 · 7, smallest | not built |
| DW-0604 | The Photo sources dialog: device switches, folders from Files | not built |
| DW-0602 | Selecting: three checked, the selection bar | not built |
| DW-0605 | One photo open: top bar, the photo, the strip, Share · Offline · Details · Trash | not built |
| DW-0605b | The original is downloading: "12.4 of 30.9 MB", the thin bar | not built |
| DW-0606 | The ⋯ menu: Save to PC… · Move… · Rename · Duplicate | not built |
| DW-0607 | Share a photo: the dialog | not built |
| DW-0608 | Offline: "On this PC", the message | not built |
| DW-0609 | Details: the dialog | not built |
| DW-0610 | Trashed: the next photo, "Moved to Trash" with Undo | not built |
| DW-0611 | A video playing: Windows' player controls drawn for reference | not built |

The brief also names states the board does NOT draw for this group. No frame id exists for them, so there is
no golden to gate against; each is listed in the PR body under "frames NOT built":
albums, loading placeholders, a thumbnail that is not there yet, the offline state, the error state, and a
comparison tool.
