# Frames: what is open

Kept by the integrator (I2). First written on 2026-11-12 03:30 CET as `docs/wave-1-open.md`, with the last of the
four wave-1 pull requests (#291 sign-in, #292 settings, #293 shared and trash, #294 files). Renamed and brought up
to date on 2026-11-12 08:10 CET with the last pull request of the second batch (#304 settings rest; before it #298,
#299, #300, #305, #306, #297, #303, #302).

**Sections 1, 2 and 2a to 2c are measured on the tree of #304** (branch `native/f-settings-rest` at 6682d809, which
holds `v2-native` f34a8eee and #302). **Sections 3 to 8 are as written after wave 1 and were NOT re-checked for this
update**: some of what they ask for has been built since (the conflict dialog, Bandwidth aside). Where a line is a
lane's claim that I did not check myself, it says "(lane)". Rune is pinned at `c92e462e`.

## 1. Frames: built and not built

Ten checklists list 310 different frames: 197 are built (each is a scene of the app, `beebeeb-drive
--list-scenes` prints 197) and 113 are not. "Built" here means: the frame id is a scene. It says nothing about its
gate (section 2).

| Group | Checklist | Listed | Built | Not built |
| --- | --- | --- | --- | --- |
| Sign-in, gate, first run | `src/frames/signin/FRAMES.md` | 70 | 41 | 29 |
| The account inside settings | `src/frames/account/FRAMES.md` | 29 | 29 | 0 |
| Settings | `src/frames/settings/FRAMES.md` | 50 | 28 | 22 |
| Shared, Search, Trash | `src/frames/shared/FRAMES.md` | 26 | 13 | 13 |
| Files | `src/frames/files/FRAMES.md` | 50 | 22 | 28 |
| Global elements, Windows-only frames | `src/frames/global/FRAMES.md` | 16 | 13 | 3 |
| Photos | `src/frames/photos/FRAMES.md` | 13 | 13 | 0 |
| This PC (Sync history, Available offline) and Backups | `src/frames/this_pc/FRAMES.md` | 16 | 15 | 1 |
| The b, its flyout and its menus | `src/frames/tray/FRAMES.md` | 26 | 21 | 5 |
| Open files (viewers) | `src/frames/viewers/FRAMES.md` | 23 | 11 | 12 |
| Total, each frame once | | 310 | 197 | 113 |

The rows add up to 319 listed and 206 built: nine frames stand on both the Files and the Global checklist (DW-0101,
0102, 0107, 0108, 0109, 0114, 0118, 0119, 0121; all nine built: six by the global lane, three by the Files lane).
A frame of the board that is on none of the ten checklists is in neither column, and I did not look for those.

**Not built (113):**
- **Sign-in (29):** DW-A11 to A17 (7, approving a sign-in); DW-A31 to A44 (14, create an account; no lane
  assigned); DW-0009, DW-0014; DW-0380, DW-0301 to DW-0305 (6, the first-time guide). No goldens for any (lane).
- **Settings (22):** DW-0708, 0716, 0716b, 0744, 0744b, 0745, 0784, 0784b, 0784c, 0784d, 0785, 0785b to 0785f,
  0786, 0786b to 0786d, 0790, 0790b.
- **Shared, Search, Trash (13):** the dialogs DW-0504, 0511, 0512, 0512b, 0513, 0513b, 0514, 0515, DW-0749; Search
  DW-0801, 0802, 0802b, 0803.
- **Files (28):** DW-0403, 0407, 0408, 0408b, 0411, 0412, 0413, 0415, 0416, 0420, 0420b, 0421 to 0427, 0429, 0480,
  0481, DW-1102, 1103, 1105, 1107 to 1110.
- **Global (3):** DW-1183, DW-1901, DW-1902.
- **This PC and Backups (1):** DW-0738.
- **Tray (5):** DW-1201, 1230, 1231, 1231b, 1234.
- **Viewers (12):** DW-1501 to 1506, 1514, 1515, 1520, 1521, 1522, and DW-1507 (the entry state).

## 2. Gate lines that are not at 0 failing

Whole-tree gate, once, on 6682d809: every one of the 197 scenes, light and dark, `scripts/compare.sh <id>
[--theme dark] --no-build` = 394 lines. **270 lines at 0 failing on all six counts, 124 not.** Every line printed
its six counts. No tolerance was raised and no key dropped or special-cased for any of them.

| Count | Lines not at 0 | Failing checks | Checked |
| --- | --- | --- | --- |
| geometry | 64 | 692 | 42,662 |
| pixels | 118 | 1,162 | 42,662 |
| text | 8 | 26 | 17,488 |
| icons | 0 | 0 | 10,072 |
| fills | 4 | 4 | 6,946 |
| frame tiles | 69 | 102,940 | 17,278,304 |

The 124 lines fall in three parts: 56 are frames in the Files window (2a), 6 are the Windows Hello layer (2b), 62
are the lanes' own open lines (2c).

Keymap scope, once over every frame (`scripts/check-keymap-scope.sh`, 197 frames, light and dark): 394 captures,
394 "the same set", 0 differing.

### 2a. The Files window: 56 lines, 28 frames. Board must be updated: toolbar button + Free up space

Since #305 (Files rest, eed8c3d0) the Files toolbar has an "Open in File Explorer" button and the window offers
Free up space; the board draws neither. That moves ten toolbar keys (`panel-icon`, `panel-toggle-btn`, `sort-btn`,
`sort-caret`, `sort-icon`, `view-grid`, `view-grid-icon`, `view-list`, `view-list-icon`, `view-toggle`) and with
them the `toolbar` box in EVERY frame that shows the Files window: geometry 10, pixels 11 in the plain case. The
lead's ruling: the board is updated (toolbar button + Free up space), the code is not; nobody chases these lines
until then, and a frame gate that needs a control uses a frame outside the Files window (DW-A51).

18 frames carry only those keys, in both themes: DW-0101, DW-0102, DW-0103, DW-0104, DW-0105, DW-0106, DW-0107, DW-0108, DW-0109, DW-0114, DW-0118, DW-0119, DW-0401, DW-0402, DW-0405, DW-0414, DW-1101, DW-1106.

10 frames carry more than those keys. Whether the rest is a consequence of the same shift (a menu or a dialog
hanging from a moved button) or a line of its own was NOT separated; it has to be read again once the board is
updated:

| Frame | Theme | Failing | Keys beyond the toolbar's |
| --- | --- | --- | --- |
| DW-0121 | light | geometry 10, pixels 12, frame tiles 40 | 1: `scrim` |
| DW-0121 | dark | geometry 10, pixels 11, frame tiles 9 | 1: `scrim` |
| DW-0404 | light | geometry 34, pixels 67, frame tiles 810 | 56: `detail-added` `detail-changed` `detail-changed-on` `detail-kind` `detail-name` `detail-pane` `encrypted-badge` `encrypted-text` `lock-icon` `menu` `menu-item-0` `menu-item-0-label` `menu-item-1` `menu-item-1-check` ... |
| DW-0404 | dark | geometry 34, pixels 65, frame tiles 566 | 54: `detail-added` `detail-changed` `detail-changed-on` `detail-kind` `detail-name` `detail-pane` `encrypted-badge` `encrypted-text` `lock-icon` `menu` `menu-item-0` `menu-item-0-label` `menu-item-1` `menu-item-1-check` ... |
| DW-0406 | light | geometry 16, pixels 18, frame tiles 207 | 7: `detail-pane` `share-box` `share-btn` `share-icon` `versions-box` `versions-btn` `versions-icon` |
| DW-0406 | dark | geometry 16, pixels 18, frame tiles 200 | 7: `detail-pane` `share-box` `share-btn` `share-icon` `versions-box` `versions-btn` `versions-icon` |
| DW-0409 | light | geometry 10, pixels 12, frame tiles 42 | 1: `scrim` |
| DW-0409 | dark | geometry 10, pixels 11, frame tiles 9 | 1: `scrim` |
| DW-0410 | light | geometry 11, pixels 16, frame tiles 77 | 5: `rename-caret` `rename-field` `rename-text` `row-6` `rows` |
| DW-0410 | dark | geometry 11, pixels 16, frame tiles 66 | 5: `rename-caret` `rename-field` `rename-text` `row-6` `rows` |
| DW-0410b | light | geometry 11, pixels 16, text 1, fills 1, frame tiles 64 | 5: `rename-caret` `rename-field` `rename-text` `row-7` `rows` |
| DW-0410b | dark | geometry 11, pixels 16, text 1, fills 1, frame tiles 57 | 5: `rename-caret` `rename-field` `rename-text` `row-7` `rows` |
| DW-0417 | light | geometry 10, pixels 12, frame tiles 42 | 1: `scrim` |
| DW-0417 | dark | geometry 10, pixels 13, frame tiles 9 | 3: `field-0-caret` `row-8` `scrim` |
| DW-1104 | light | geometry 28, pixels 40, text 10, frame tiles 740 | 29: `detail-pane` `menu` `menu-item-10` `menu-item-10-label` `menu-item-11` `menu-item-11-label` `menu-item-11-shortcut` `menu-item-5` `menu-item-5-label` `menu-item-6` `menu-item-6-label` `menu-item-6-shortcut` `menu-item-7` `menu-item-7-label` ... |
| DW-1104 | dark | geometry 28, pixels 39, text 10, frame tiles 597 | 28: `detail-pane` `menu` `menu-item-10` `menu-item-10-label` `menu-item-11` `menu-item-11-label` `menu-item-11-shortcut` `menu-item-5` `menu-item-5-label` `menu-item-6` `menu-item-6-label` `menu-item-6-shortcut` `menu-item-7` `menu-item-7-label` ... |
| DW-1232 | light | geometry 12, pixels 19, frame tiles 44 | 8: `row-4` `row-4-size` `row-5` `row-5-size` `rows` `sc-1-2-home` `sc-1-4-home` `sc-dlg` |
| DW-1232 | dark | geometry 12, pixels 20, frame tiles 12 | 10: `row-0-tile` `row-4` `row-4-size` `row-5` `row-5-tile` `row-6-tile` `rows` `sc-1-2-home` `sc-1-4-home` `sc-dlg` |
| DW-1233 | light | geometry 11, pixels 18, frame tiles 50 | 7: `row-7` `row-7-size` `rows` `sc-1-0-home` `sc-1-1-home` `sc-2-1-home` `sc-dlg` |
| DW-1233 | dark | geometry 11, pixels 20, frame tiles 13 | 10: `row-4` `row-4-tile` `row-6-tile` `row-7` `row-7-size` `row-8` `rows` `sc-1-0-home` `sc-2-1-home` `sc-dlg` |

### 2b. The Windows Hello layer: 6 lines

DW-A04, DW-A05 and DW-A29, light and dark: pixels 26 to 41 and frame tiles 2,159 to 30,293 failing per line. The
board golden holds Windows' own dialog and its scrim over the frame. **Ruling (lead, on #291): recapture the golden
with that layer hidden.** Not done yet.

### 2c. The lanes' own open lines: 62 lines, 41 frames

The keys are the files in the gate's `fail/` folder of that line.

| Frame | Theme | Failing | Keys | Cause as far as known |
| --- | --- | --- | --- | --- |
| DW-A51 | dark | pixels 1 | `item-0-0-avatar` | not solved (lane f-account) |
| DW-0206 | light | pixels 1 | `nav-2-b` | the b's plate (lane) |
| DW-0206 | dark | pixels 1 | `nav-2-b` | the b's plate (lane) |
| DW-0209 | dark | pixels 1 | `cmp-opt-5` | tile 27 > 26 (lane) |
| DW-0428 | light | pixels 1 | `status-dot` | small-disc rim (edge 9 > 9) |
| DW-0502 | light | pixels 1 | `item-0-2-pill-dot` | small-disc rim |
| DW-0502 | dark | pixels 1 | `item-0-2-pill-dot` | small-disc rim |
| DW-0506 | light | pixels 1 | `item-0-2-pill-dot` | small-disc rim |
| DW-0506 | dark | pixels 1 | `item-0-2-pill-dot` | small-disc rim |
| DW-0602 | light | pixels 3 | `ph-tile-0-0-check` `ph-tile-0-3-check` `ph-tile-1-2-check` | the checked circle's top row (lane) |
| DW-0604 | light | pixels 1 | `ph-item-0-1-switch` | one switch, edge 9.0 > 9 (lane) |
| DW-0605 | light | pixels 6, frame tiles 1 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `status-details` | the four action labels (white text), the title (lane) |
| DW-0605 | dark | pixels 5 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` | the four action labels (white text), the title (lane) |
| DW-0605b | light | pixels 5, frame tiles 1 | `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `status-details` | as DW-0605 |
| DW-0605b | dark | pixels 4 | `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` | as DW-0605 |
| DW-0606 | light | pixels 6, frame tiles 1 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `status-details` | as DW-0605 |
| DW-0606 | dark | pixels 5 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` | as DW-0605 |
| DW-0607 | light | pixels 3 | `ph-act-0` `ph-act-2` `ph-act-3` | the action labels behind the scrim (lane) |
| DW-0608 | light | pixels 6, frame tiles 1 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `status-details` | as DW-0605 |
| DW-0608 | dark | pixels 5 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` | as DW-0605 |
| DW-0609 | light | pixels 3 | `ph-act-0` `ph-act-2` `ph-act-3` | as DW-0607 |
| DW-0610 | light | pixels 7, text 1, frame tiles 1 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `ph-toast-action` `status-details` | as DW-0605, and "Undo Ctrl+Z" (lane) |
| DW-0610 | dark | pixels 6, text 1 | `folder-title` `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `ph-toast-action` | as DW-0605, and "Undo Ctrl+Z" (lane) |
| DW-0611 | light | geometry 4, pixels 10, text 1, frame tiles 81 | `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `ph-photo` `ph-player` `ph-player-text-1` `ph-player-text-2` `ph-player-track` `ph-stage` | built without the board's note to the reader; the player (lane: not gated) |
| DW-0611 | dark | geometry 4, pixels 10, text 1, frame tiles 80 | `ph-act-0` `ph-act-1` `ph-act-2` `ph-act-3` `ph-photo` `ph-player` `ph-player-text-1` `ph-player-text-2` `ph-player-track` `ph-stage` | built without the board's note to the reader; the player (lane: not gated) |
| DW-0713 | light | pixels 1 | `slider-knob` | the slider's knob (lane f-settings-rest) |
| DW-0713 | dark | pixels 1 | `slider-knob` | the slider's knob (lane f-settings-rest) |
| DW-0714 | light | pixels 1 | `slider-knob` | the same |
| DW-0714 | dark | pixels 1 | `slider-knob` | the same |
| DW-0737 | dark | pixels 3 | `item-0-3-knob` `item-0-4-knob` `item-0-5-knob` | three switch knobs (lane f-backups-history) |
| DW-0742 | light | pixels 1 | `code-2` | the lit box's stroke on a half pixel, edge 32 > 9 (lane) |
| DW-0742 | dark | pixels 1 | `code-2` | the lit box's stroke on a half pixel, edge 32 > 9 (lane) |
| DW-0783 | light | pixels 7 | `insbar-0` `insbar-0-fill` `insbar-1` `insbar-1-fill` `insbar-2-fill` `meter-0` `meter-0-part-0` | the meter and the insight bars (lane f-settings-rest) |
| DW-0783 | dark | pixels 4 | `insbar-0` `insbar-0-fill` `meter-0` `meter-0-part-0` | the meter and the insight bars (lane f-settings-rest) |
| DW-0787 | dark | pixels 1 | `item-0-0-pop` | an edge check through overlaid text |
| DW-0789b | dark | pixels 1 | `item-1-1` | the same |
| DW-1301 | light | pixels 1 | `status-dot` | small-disc rim |
| DW-1302 | light | pixels 1 | `status-dot` | small-disc rim |
| DW-1303 | light | pixels 1 | `status-dot` | small-disc rim |
| DW-1304 | light | geometry 1, pixels 5, fills 1, frame tiles 4 | `search-box` `search-caret` `search-placeholder` `status-dot` `toolbar` | the caret: a headless render has no focus (lane) |
| DW-1304 | dark | geometry 1, pixels 4, fills 1, frame tiles 3 | `search-box` `search-caret` `search-placeholder` `toolbar` | the caret: a headless render has no focus (lane) |
| DW-1512 | light | pixels 4 | `line-15` `seg` `seg-0` `viewer-body` | a glyph of the path under the control's edge, one line's tile (lane) |
| DW-1512 | dark | pixels 2 | `seg` `seg-0` | a glyph of the path under the control's edge, one line's tile (lane) |
| DW-1523 | light | pixels 1 | `crumb-dot` | the unsaved dot's edge (lane) |
| DW-1523 | dark | pixels 1 | `crumb-dot` | the unsaved dot's edge (lane) |
| DW-1525 | light | geometry 4, pixels 6, frame tiles 31 | `dlg-btn-0` `scrim` `tb-btn-0` `tb-btn-0-icon` `tb-btn-0-label` `toolbar` | the toolbar under the scrim (the board draws it without the dot and without Ctrl+S, against DW-1523) and the Save button's edge (lane) |
| DW-1525 | dark | geometry 4, pixels 6, frame tiles 10 | `dlg-btn-0` `scrim` `tb-btn-0` `tb-btn-0-icon` `tb-btn-0-label` `toolbar` | the toolbar under the scrim (the board draws it without the dot and without Ctrl+S, against DW-1523) and the Save button's edge (lane) |
| DW-1526 | light | geometry 4, pixels 5, frame tiles 31 | `scrim` `tb-btn-0` `tb-btn-0-icon` `tb-btn-0-label` `toolbar` | the toolbar under the scrim, as DW-1525 (lane) |
| DW-1526 | dark | geometry 4, pixels 5, frame tiles 10 | `scrim` `tb-btn-0` `tb-btn-0-icon` `tb-btn-0-label` `toolbar` | the toolbar under the scrim, as DW-1525 (lane) |
| DW-A07 | light | pixels 2 | `status-dot` `status-ring` | the ring one device pixel further left; the dot is the small-disc rim |
| DW-A07 | dark | pixels 2 | `status-dot` `status-ring` | the ring one device pixel further left; the dot is the small-disc rim |
| DW-A19 | dark | pixels 1 | `card-0-avatar` | `card-0-avatar` in dark, not solved |
| DW-A20 | dark | pixels 1 | `card-0-avatar` | the same |
| DW-A21 | dark | pixels 1 | `card-0-avatar` | the same |
| DW-A22 | dark | pixels 1 | `card-0-avatar` | the same |
| DW-A28 | dark | pixels 1 | `field-0-caret` | the board's caret starts on half a device pixel (lane) |
| DW-A63 | dark | pixels 1 | `card-0-avatar` | the same |
| DW-A65 | dark | pixels 1 | `field-0-caret` | the last letter beside the caret is drawn heavier, light on dark (lane) |
| DW-A72 | light | pixels 1 | `digit-2` | the inner line of a focused box on a half device pixel (lane) |
| DW-A72 | dark | pixels 1 | `digit-2` | the inner line of a focused box on a half device pixel (lane) |
| DW-A78 | light | pixels 1 | `digit-2` | as DW-A72; dark also the confirm's bold line over the list's edge (lane) |
| DW-A78 | dark | pixels 3 | `digit-2` `item-2-0` `list-2-body` | as DW-A72; dark also the confirm's bold line over the list's edge (lane) |

Causes that want ONE decision each, not a fix per frame:
- **the small-disc rim** (Rune limitation 1): `status-dot` and `item-0-2-pill-dot` in DW-0428, 0502, 0506, 1301,
  1302, 1303, DW-A07's dot (10 lines); DW-0791, red for this after wave 1, is at 0 now;
- **an avatar in dark** (`card-0-avatar`, `item-0-0-avatar`): DW-A19, A20, A21, A22, A63, A51 (6 lines), not solved;
- **a caret or a focused box on half a device pixel**: DW-A28, A72, A78, DW-0742 (7 lines);
- **an edge check beside or through light-on-dark text** (two text rasterisers): DW-0787, DW-0789b, DW-A65 (3 lines);
- **Photos' open photo** (`ph-act-0` to `ph-act-3`, the title): DW-0605, 0605b, 0606, 0607, 0608, 0609, 0610, 0611
  (14 lines).

Correction of this file's first version: it said three causes covered "13 of the 19 lines" of wave 1; the lines it
then listed add up to 15 (9 small-disc rim, 4 Windows Hello, 2 overlaid text).

## 2d. Tests

- **One test hangs on the Linux build VM.** `source_engine::tests::timings_and_query_plan_for_5000_children_in_a_70000_row_db`
  (`src/source_engine_tests.rs`, added by #280): it builds a 5,204-row and a 70,000-row database, prints the time
  of a 500-row page and of the whole 5,000-child folder, and asserts SQLite's plan for the listing query (no time
  is asserted). On the loaded 16-core build VM it did not finish in over 25 minutes (during #306); on the Legion
  it passes within the suite's 30 to 40 seconds. Since then the VM run skips it by exact name and the Legion run
  includes it. Not fixed, not investigated.
- On the tree of #304: the Linux VM, that test skipped: 534 passed, 0 failed, 1 filtered out; the Legion, nothing
  skipped: 535 passed, 0 failed. The identity pin (`crates/beebeeb-engine`, filter `identity_pin`): 6 passed, 0
  failed.
- Legion only: the engine's `ipc_socket::framing_tests::changes_and_thumbnail::fetch_thumbnail_reports_daemon_side_failures_without_writing_the_dest`
  fails there ("no delimited reply within 5s", 3 of 3) and passes in CI's Linux job on the same engine code. Known
  under load (lead); CI decides.
- Not run in this batch: `cargo test --features feedback`; the Windows build VM (CI's "Native Rune app (Windows
  build)" job covers the compile on Windows, nothing ran the native tests there).

## 2e. Deviations from the board

- **The flyout's accessible name.** The board's Accessibility note says "a dialog named Beebeeb". The app names it
  from its identity (`AppIdentity::flyout_window_title()`): "Beebeeb" in the real build, "Beebeeb Test" in a test
  build. Ruling (lead, 2026-11-12): two apps side by side must be tellable apart by a screen reader; the board's
  note describes the real app. `frames/tray/view.rs`, `tray::tests::names_and_roles`.
- **The Files toolbar** has a button and a command the board does not draw (2a): there the board changes, not the
  code.

## 2f. The app names itself from the build's identity: open

Ruling (Guus, 2026-11-12 08:00, relayed by the lead): "use the build name dynamicly". Every place the app names
ITSELF takes the name from the build's identity (`src/shell/identity.rs`), not from a literal. Done so far: the
window titles (`shell/mod.rs`), the tray's tooltip, the flyout's accessible name (2e). Still literals, seen by a
search for `Beebeeb` in production text on the tree of #304 (not a complete audit: 181 string literals hold the
word, most of them are sentences of the board's copy, and I read only the ones in a label, a title or a name):

| Where | Literal | What it is |
| --- | --- | --- |
| `src/ui/caption.rs:42` | `"Beebeeb Drive"` | the DRAWN title in the caption bar (key `cap-title`: the gate compares its text with the board's) |
| `src/ui/window.rs:45`, `src/frames/files/window.rs:280`, `src/frames/shared/parts/chrome.rs:47`, `src/frames/photos/chrome.rs:313`, `src/frames/viewers/chrome.rs:338`, `src/frames/this_pc/mod.rs:337`, `src/ui/au.rs:230` | `"Beebeeb Drive"` | the window's accessible name, seven copies |
| `src/frames/settings/mod.rs:668`, `src/frames/account/mod.rs:509` | `"Beebeeb Drive, Settings"` | the window's accessible name in settings |
| `src/frames/backups/view.rs:564` | `"Beebeeb Drive, Settings, Backups"` | the same, Backups |
| `src/ui/kit.rs:72` | `"Beebeeb"` | the logo's accessible name (on the pin's allow-list as "the brand, for both builds") |
| `src/ui/au.rs:161`, `src/ui/setup.rs:72`, `:240` | `format!("Beebeeb {product}")` | the brand line's accessible name |
| `src/ui/au.rs:204` | `"About Beebeeb"` | a link's accessible name |
| `src/frames/settings/source.rs:324` | `product: "Beebeeb Drive"` | the product name in the version facts (About, the support bundle) |
| `src/frames/tray/commands.rs:10`, `:98` | `"Beebeeb"`, `"Beebeeb Drive"` | the board's command table: the app's own menu and its window, as headings |

Two questions came with this list: a drawn name that follows the identity makes a test build's frame differ from
the board in that text (`BEEBEEB_APP_IDENTIFIER` unset is the TEST app), and the identity pin sees only a literal
that IS a name (`"Beebeeb"`, `"Beebeeb Test"`, ...), not `"Beebeeb Drive"`: all but two rows above pass it today.

**Rulings (lead, 2026-11-12):**
1. **Drawn names come from the identity, and the gate keeps comparing against the board.** The headless snapshot
   path (`--snap`, which opens no engine and no credential store) draws with the REAL display name; a live TEST
   build draws "Beebeeb Test". One test pins each half: the snapshot shows the board's name; a live TEST build's
   window title is "Beebeeb Test". Display name only: the snapshot never takes REAL's folders, service names or
   identifiers.
2. **The pin seeing only exact names** goes to the engine lane with review P2-5; it is not the integrator's.
3. **The conversion of the rows above is ONE follow-up pull request**, after the maps pull request and not part of
   it. Not started.
4. **The ten Files-window frames with more red keys than the toolbar's (2a)** stay as listed; they are separated
   when the board is updated.

## 3. Requests to the design-system lane (`src/theme/**`, `src/ui/**`)

Tokens:
- `--color-red-text`, `--color-green-text` (asked by sign-in `src/ui/au.rs` and by shared `src/frames/shared/parts/`).
- `red_deep` (`--red-deep`): `menu::danger_ink` in `src/ui/menu.rs` is a literal colour (files; review P3.8).
- `warn` (`--mk-warn`), `scrim`, `glass` (settings).
- The selected grouped-list item: ink 6 % light, 9 % dark (`t.selection` is 9 % in both); the menu shadow
  `0 8px 28px` black 20 % (`t.shadow_menu` is 22 %) (shared).
- `type_au` (17 type styles) and the primary button's hover and press colours live in `src/ui/au.rs`; they belong
  in `theme/tokens.rs`. `scripts/gen-tokens.py` does not produce the current `tokens.rs`.

One source for "is the window dark" (settings, review P2.5d, NOT fixed): choosing Dark in DW-0787 darkens settings
only. `ui/window.rs`, `shell/placeholder.rs`, the dialog scrim in `shell/registry.rs` and the flyout each read
`cx.system_dark()` themselves; they need ONE `dark(s, cx)`.

Caption buttons are Tab stops (settings): under `Cx::web_focus` the three caption buttons (`src/ui/caption.rs`,
`caption_button`) take focus before the content; Windows' own do not. One `.focusable(false)` there fixes every
frame that turns `web_focus` on (sign-in and settings both do). Settings' test pins it as known (`stops[..3]`).

Forked components to fold into `src/ui/` (each exists two or three times now):
| Piece | Copies |
| --- | --- |
| Sidebar with counts from data and actions on every row | `src/ui/sidebar.rs` (constants; Trash is a constant 12, DW-0405 needs 13), `src/frames/files/sidebar.rs`, `src/frames/shared/parts/sidebar.rs`, and settings' sidebar rows in `src/ui/settings_shell.rs` |
| Toolbar (Sort and New open menus, Back and Forward state, path title, search field of its own) | `src/ui/toolbar.rs`, `src/frames/files/toolbar.rs`, `src/frames/shared/parts/toolbar.rs` |
| In-window menu | `src/ui/menu.rs` (files), `src/frames/shared/parts/menu.rs` (fit to the window, one lit item), settings' menus in `src/ui/settings_controls.rs` |
| Grouped list and its rows | `src/ui/settings_list.rs`, `src/frames/shared/parts/list.rs`, sign-in's `au::list` / `au::item` |
| Empty state | `src/ui/empty.rs` (files), `src/frames/shared/parts/empty.rs` |
| Status line with words from the caller | `src/ui/status_bar.rs`, `src/ui/status_line.rs` (files), `src/frames/shared/parts/chrome.rs` |
| Buttons, pills, worded segments | `src/ui/button.rs` (files), `src/frames/shared/parts/controls.rs`, `au::button` / `au::pill`, `src/ui/settings_controls.rs` |
| Switch | `detail_pane::switch` (knob white in both states; the board draws paper colour when on), `settings_list::switch` (right), files' switch card with a handler |
| Avatar | `kit::avatar` (10 / 8 px initials, fixed key `initials`), `au::avatar` (15 / 11 px, a key per avatar) |
| File table | `src/ui/file_list.rs`, `src/frames/files/list.rs` (sorting headers, role `row`, one Tab stop), `src/frames/shared/parts/table.rs` (named columns) |
| The Files window and its keys | `ui::window::files_window` and `ui::keys::on_key` are superseded by `frames::files`; kept alive by a `const _` in `src/frames/files/mod.rs`. Remove once folded |

Others:
- `au::pill` hard-codes the key `status-dot`: a second dotted pill in one frame doubles the key (sign-in, review P3-4).
- `detail_pane` rows keyed for a multi-selection; `ui/gate.rs` to export the DRAWN (rotated) bounding box (files, for DW-0403).
- `capture-golden.mjs` exits 1 on the Trash frames (`no text node for col-pc`: a header cell without words)
  though it writes the export (shared).

### The gate's key names: one scheme is still owed

Four lanes keyed the same board classes (`.kit-btn`, `.kit-list`, `.kit-pill`, `.wn-menu`, `.kit-empty`) under
four naming schemes: `btn-<words>` (shared), `btn-<n>` (settings, sign-in), `empty-add` (files); `mi-<words>`
(shared), `menu-item-<n>` (files), `menu-<m>-item-<n>` (settings). After the merges every section ran in every
frame, and a recapture exported keys the native tree does not have (`scripts/check-keymap-scope.sh`: DW-0402 254
committed, 287 captured; DW-0505 162 / 179; DW-0787 107 / 115; DW-0701 99 / 103; DW-A02 58 / 66; DW-0508 did not
capture). Integration scoped each section to its group (`groupOf` in `scripts/keymap.js`: settings by `.mk-set`,
account by `.au-mac`, shared-trash by the selected sidebar item, files otherwise). That keeps the committed
goldens true; it is not the design. Wave 2 wants ONE scheme per component, in the component (the key comes with
`ui::menu`, `ui::button`, ...), and the keymap in one file per component, not per lane. Until then: a new group
adds its `groupOf` value and runs `check-keymap-scope.sh` over EVERY built frame before it calls its keys done.
A Search, Photos or Backups frame falls into `files` today: wrong as soon as one is built.

## 4. Requests to the shell lane (`src/shell/**`)

- Drain the intent queues; nothing reads them today: `State::frames.intents` (shared, trash: 16 controls),
  `SettingsUi::take_intents()` (21 controls, at most 32 kept), `Actions::intents` of files (all but DW-0701,
  DW-1301, DW-0715), sign-in's `Intent`s (5 controls).
- `shell/host.rs` `poll`: `changed |= crate::frames::signin::poll(s, tx);` (an engine answer that arrives between
  presses is applied only at the next press).
- `shell/mod.rs` `main_app`: `.on_focus(|s, focused, _| crate::frames::signin::on_window_focus(s, focused))` (a
  shown password goes back to bullets when the window loses focus). A real blur or capture exclusion of the field
  is a window attribute in `shell/platform/windows.rs`: not built.
- A route hook per registered frame (`on_route`), instead of the one line `host::dispatch` →
  `frames::settings::sync`; `Shell::refresh` (a change of account) does not pass through it.
- Esc and Ctrl+, are taken by the shell before the frame; Alt+Left / Alt+Right too, and the route machine does not
  know Files' folders: in the app those keys do not leave a folder (`shell/README.md`: "Files does not follow the
  route yet").
- `SettingsPane` has no Bandwidth (DW-0785). `Shell::toggle_flyout` is private (the status line asks for DW-1201).
- `AppMachine::at(route)`: start on a section with empty history, so scenes can drop `frames::Ui::first_place`.
- The registry lists Settings(General) and Settings(Notifications) only; every other pane a settings frame
  belongs to still needs its line.
- Feedback mode: only Files is `Allowed`. Shared, Trash and both settings groups are `Denied("its lane has not
  opted in yet")`; sign-in is `Denied("sign-in shows your email and takes your password")` and stays so until the
  pairing code and the QR sit in a `secure-` container (`src/feedback/README.md`, duty 1).

## 5. Engine calls the frames need (the wiring list for the engine lane)

Each group draws from a trait with a board fixture; a shipped build must get the engine's implementation.
Settings already starts on `Unwired` outside a development build (`frames/settings/source.rs`, `source::first`);
files' `Actions::default()` is `NoEngine`; **shared and trash still start on the board's sample data**, and
sign-in's `signin::start()` gives no shared session.

| Trait (file) | Calls |
| --- | --- |
| `SignInSource` (`src/frames/signin/source.rs`), mirrors `src-tauri/src/auth_v2/gate.rs` | `begin_sign_in(email, password)` → `SignedIn(Ok(next) / Err(problem))`, `AccountDeleted`, `RetryAfter { seconds }`; `begin_hello()`, `cancel_hello()` → `Hello(Succeeded / NotRecognized / Cancelled)`; `begin_device_request()`, `cancel_device_request()` → `DeviceRequested`, `DeviceTick`, `DeviceConnection { connected }`, `Device(Approved / Denied / Expired / KeysNotReceived)`; declared for frames not built yet: `begin_two_factor`, `begin_restore`, `begin_passkey`, `begin_unlock`, `begin_recovery_verify`, `begin_set_password`; sign-up has no source yet (DW-A31) |
| `SettingsSource` (`src/frames/settings/source.rs`) | theme preference read / store; updater state, check, download; release channel store and re-check; a release's notes; clipboard copy; write the support bundle and open a mailto; storage locations list, store choice, store "only here", reload; alert switches read / store; Windows' notification permission, open Windows Settings; the account's recent notifications by day, `mark_recent_seen` |
| `SharedSource` (`src/frames/shared/`) | `accept` / `decline` an invitation; `keep_offline`; `stop_sharing`; `close_request` / `remove_request`; still intents, need a call: leave-share, the link's URL for Copy link, set-link-expiry, set-request-expiry, set-link-password, people on a share, create-file-request, reveal the local path |
| `TrashSource` (`src/frames/trash/`) | `restore`; `emptying()` progress; still intents: `purge(id)` and `purge_all()` after the authenticator code |
| `FilesSource` (`src/frames/files/source.rs:137`) | `counts`, `account`, `storage_split`, `sync_state`, `details`, `changed_label`, `can_add`, `sort_of`, `set_sort`, `trash`, `restore`, `duplicate`, `keep_offline`, `copy_link`, `open`, `show_in_file_explorer`, `upload`, `changes`; each action answers `Result<(), ActionError>` at once, later effects arrive through `changes()`. Not calls yet (their frames are not built): new folder, rename, move, share, preview, details, version history, the pickers |
| `frames::PcStatus` (`src/frames/mod.rs`) | `sync_words()`, `summary()`: the same facts as `FilesSource::sync_state` and `counts`; one of the two should go |
| Across groups | `search(query)` (the field emits `Intent::Do("search", typed)`); thumbnail pixels per photo; per item `details(id)`, `people(id)`, `reveal(id)` |

The same engine fact is asked for under two names in three places: keep offline (`SharedSource::keep_offline`,
`FilesSource::keep_offline`), restore from Trash (`TrashSource::restore`, `FilesSource::restore`), copy link
(`Intent::Do("copy-link")`, `FilesSource::copy_link`). One call each on the engine side.

## 6. Rune limitations (deduplicated)

| # | Limitation | Where in Rune | Met by | Workaround in the tree |
| --- | --- | --- | --- | --- |
| 1 | A small filled circle's anti-aliased rim differs from Chromium's by 10 to 24 levels | not located | sign-in, settings, shared: 9 red lines | none (a smaller radius would special-case a key) |
| 2 | Text is rasterised a little heavier than Chromium's light-on-dark; an edge check through overlaid text compares two rasterisers | not located | settings: 2 red lines | none |
| 3 | A span underline has no offset and runs through descenders | `crates/rune/src/text/span.rs:21` | sign-in | `au::link` draws the underline as bars with gaps |
| 4 | The element tree reports a text input's box, not its text's; and the layout box of a rotated element, not the drawn one | `ElementInfo.rect`, `crates/rune/src/runtime/elements.rs:25`; rotation `crates/rune/src/el/visuals.rs:381` | sign-in, files (DW-0403 not built for it) | sign-in: an empty marker as wide as `rune::text::advance`; files: none |
| 5 | Return does not press a focused `link` or `button` unless `Cx::web_focus()` is set | `crates/rune/src/el/focus.rs:170-176`, `crates/rune/src/runtime/cx.rs:196` | sign-in, settings | both set `web_focus` (side effect: the caption buttons, section 3) |
| 6 | An element's `on_key` runs only while focus is inside it; the window root's not at all while a modal is mounted; a focused text field's keys never reach an ancestor | `crates/rune/src/runtime/input/keys.rs:21`, `:231` | sign-in, shared, files | keys go through `shell::host::on_key` and the registry line's `on_key`; a menu carries its own `on_key`; the search field uses `on_input_key` |
| 7 | `Runtime::frame(0.0)` never finishes a newly mounted element's entrance, so a press misses it | `crates/rune/src/runtime/frame.rs:308-313` | sign-in, files (tests only) | the tests settle first (`settle_all`, `settle`) |
| 8 | `text::measure` rounds a width up to whole points; a row hanging from the right edge shifts by a fraction | Rune README "Widths round up to whole points" | shared | `controls::pill` takes the unrounded `rune::text::advance`; the worded segments still use a 9.75 pt padding |
| 9 | `clip_rounded` also cuts the element's own outside `box_shadow` | not located | settings | the ring sits on a parent, the clip one level in |
| 10 | `Runtime::focused_key` is private | n/a | shared (tests) | tests read `ElementInfo::focused` |
| 11 | Only the left and right mouse buttons are delivered; resizability is set at window creation only | `crates/rune/src/window/events.rs:461,489` | shell | `Event::Back` / `Forward` are ready; the first-run window stays resizable |

## 7. Review findings left open

- **Sign-in (#291; 0 P1, 7 P2, 11 P3):** P2-5 in the record only (DW-A31 to A44 not built, no owner); P2-6 in
  part (the app-switcher blur); P2-7 in part (tokens and `au::avatar` not folded). P3-1, 4, 5, 6, 7, 9, 10, 11
  not changed; P3-3 in part ("Open Drive" hard-codes `Drive`); P3-8 (the password stays filled on the fifth wrong
  try) wants a file in `.claude/tasks/decisions/`. Two commits on the branch carry a scratchpad path as their
  subject (`db8a6653`, `af91d9c5`): left as they are.
- **Settings (#292):** P2.5d not fixed (the single dark flag, section 3). P2.6 made safe, not wired (section 4).
  P2.11a ("Restart to update" on the board's HTML against "Download update" in frames.json and the 2026-10-24
  ruling that a PC never installs in place) and P2.11b ("Uploads wait until it is." is the lane's sentence) want
  rulings. P3.12 (duplicate components), P3.13 (radios without a radio group) not done.
- **Shared and Trash (#293; 3 of 3 P1 and 9 of 9 P2 fixed):** P3-1 to P3-6 not fixed; not built: "Drag to
  Files", "Restore with Undo", loading / error / offline states on both traits.
- **Files (#294; 1 of 1 P1 fixed):** P2.9 rest (new folder, rename, move as trait calls) waits for their frames;
  P2.10 forked components not folded (section 3); P3.4 (`board_crop_top` in the product's state), P3.6
  (`menu_point` ignores the scroll offset), P3.8 (`danger_ink`) not done. Open contradictions on the board,
  decided by the lane and to be ruled: Enter against Ctrl+Down for Open (DW-0401 notes against DW-0402's
  drawing); DW-0420's drawn order is no order the Sort menu offers.
- **From integration:** `crates/beebeeb-engine` (#290) prints 5 warnings on Linux and 12 on Windows in every
  build of this crate (2 unused imports `owner_keys.rs:3`, `:7`; 2 deprecated `encrypt_*` calls, task 2275;
  `RequestKeys` more private than its `pub` field, `engine_bridge.rs:79`; 7 items never used on Windows). The
  crate's own code is at 0. (The "Frontend" check that was red here is gone: #310 removed the job, #311 the
  React sources.)

## 8. What nobody did in wave 1

No frame was looked at in a real Windows window: every picture is a headless render (lavapipe) against the board,
and the Windows build VM ran `cargo test`, not the frame gate. No screen reader, no real Tab order in a real
window, no real drop from File Explorer, no Windows Hello prompt. `cargo test --features feedback` was run by
the settings lane only (224 passed, before its last merge), never on the merged tree.
