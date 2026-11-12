# Global elements and Windows-only frames: the frame checklist (lane f-global)

The Windows frames of the board's groups "Global elements on Windows" (13: `DM-0101` to `DM-0121` carry [W], the
same numbers as `DW-`) and "Windows-only frames" (3). "The Share menu: Save to Beebeeb", "Desktop widgets" and
"Finder" have no Windows frame (every item is [mac]; File Explorer's frames are DW-1180 to DW-1183, of which
DW-1183 is in "Windows-only frames").

Built = drawn from the group's source, its controls work, a scene by frame id, gated in light and dark with all six
gate lines at 0 failing. 10 built here, 3 built by the Files frames (PR 294), 3 not built.

| Frame | What | State |
| --- | --- | --- |
| DW-0101 | No connection: the list stays and says how old it is | built, 0 |
| DW-0102 | Some items can't sync: View, dismiss | built, 0 |
| DW-0103 | The account is nearly full: Storage, dismiss | built, 0 |
| DW-0104 | The account is full: Storage | built, 0 |
| DW-0105 | The session ended after 90 days: Sign in | built, 0 |
| DW-0106 | Some backup items failed: View, dismiss | built, 0 |
| DW-0107 | "Link copied" | built by the Files frames (PR 294, `ui::toast`), 0 |
| DW-0108 | "Moved to Trash" with Undo Ctrl+Z | built by the Files frames (PR 294), 0 |
| DW-0109 | "Couldn't rename. Check your connection." with Try again | built by the Files frames (PR 294), 0 |
| DW-0114 | A new account has no files yet | built, 0 |
| DW-0118 | The first list loads in the background: the slim strip | built, 0 |
| DW-0119 | Thumbnails arrive one by one: plain tiles in the grid (`ui::grid_tiles`, `grid.rs`); the view switch works, each folder keeps its view | built, 0 |
| DW-0121 | Offline, and the file isn't on this PC: the preview says so | built, 0 |
| DW-1901 | Install Beebeeb on Windows: the installer's progress page | not built: "Windows draws it" (the NSIS installer's own pages); Beebeeb supplies its name and icon (the packaging lane, PR 283) |
| DW-1902 | The installer's last page | not built: "Windows draws it", as DW-1901 |
| DW-1183 | Beebeeb's actions in File Explorer's classic menu | not built: "Windows draws the menu"; Beebeeb supplies four shell verbs (Share with Beebeeb…, Copy Beebeeb link, Show in Beebeeb, Version history…), which are a shell registration of the engine lane, not a Rune frame |

The board has no Windows frame in this group for an update that is ready (the b's flyout, DW-1210), the lock
screen (DW-0009, the sign-in group) or the shortcuts list (DW-1232, the shell's dialog): `GlobalStatus` carries
`update_ready` and `locked` for the shell, and no banner shows while the app is locked.
