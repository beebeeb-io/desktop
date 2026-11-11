# Beebeeb Drive — Rune native UI

Native Windows desktop app for Beebeeb Drive, built with [Rune](https://github.com/anomalyco/rune).

## Build

```sh
# Source lavapipe for headless rendering on WSL
source ~/code/bb-legion-native/lavapipe/env.sh

# Shared target dir (warm cache)
export CARGO_TARGET_DIR=~/code/bb-worktrees/target-native-n2

# Run (opens window on Windows, headless on WSL)
cargo run -p beebeeb-drive-rune

# Headless snapshot
cargo run -- --snap DW-0401.png --scene DW-0401 --size 1278x798 --scale 2 --theme light

# Tests
cargo test -p beebeeb-drive-rune
```

## The frame gate

One command renders a frame headless and compares it with the board, element by element:

```sh
scripts/compare.sh DW-0401                 # light
scripts/compare.sh DW-0401 --theme dark    # dark
```

Its last line is the truth line. A frame is done when all six counts of failures are 0, in both themes:

```
DW-0401 geometry: 220 checked, 0 failing · pixels: 220 checked, 0 failing · text: 97 checked, 0 failing · icons: 51 checked, 0 failing · fills: 44 checked, 0 failing · frame tiles: 63481 checked, 0 failing
```

| Check | What it compares, per keyed element | A failure reads |
| --- | --- | --- |
| geometry | the box: x, y, w, h within 1 px (a text: x, vertical centre, and w within 2 px) | `versions-box  x 1107.5 (board 1109.8); w 130.5 (board 132.4)` |
| pixels | the pixels inside the board's box, cropped from both images: `tile` (blurred, 8 px tiles, worst tile) and `edge` (brightness across each edge: a hairline that is missing or extra) | `search-box  edge 20 > 9` · `row-7-tile  tile 145 > 26` |
| text | the string, family, size, weight, letter spacing, colour | `row-9-size  text '22.0 MB' (board '22.6 MB')` |
| icons | which glyph (the SVG path data), outlined or filled, its state name, colour | `row-6-icon  glyph doc-minimal (board path M13 2H6a2 2…)` |

`MISSING in the native tree` means the board has the key and the native tree does not: give the element that key.

What it writes (to `$EVIDENCE_DIR/<frame>[-dark]/`): `<frame>-side-by-side.png`, `<frame>-tiles.png` (every tile of
the whole frame that differs, keyed or not: **look at it; whatever shows there and has no key gets a key**),
`fail/<key>.png` (board | native, enlarged) per pixel failure, and `<frame>-gate.txt` (the truth line).

Exit 0: all green. Exit 1: differences. Exit 2: a step before the comparison failed (no golden, the build, the
render): the previous render is deleted first, so there is never a pass on a stale picture.

**Never make a red go away by raising a tolerance, removing a key or special-casing one.** Fix the component or the
token. A tolerance may only get tighter. If Rune cannot draw something, say so in the PR under "Rune limitations".

`compare.sh` builds through `scripts/coord/build-slot.sh` (set `BUILD_SLOT` if yours lives elsewhere, or pass
`--no-build` after your own build) and renders with lavapipe when `~/code/bb-legion-native/lavapipe/env.sh` exists.

### How the golden is made

```sh
node scripts/capture-golden.mjs DW-0401          # golden/DW-0401-board.png + golden/DW-0401.json
node scripts/capture-golden.mjs DW-0401 --dark   # golden/DW-0401-dark-board.png + golden/DW-0401-dark.json
```

Chromium (Playwright; `PLAYWRIGHT_DIR` names a `node_modules` that has it) opens the board at a real 200 % device
scale, with text measured as Windows and macOS measure it (fractional advances, no hinting), and exports the frame's
client area (the board's 1 px window frame is the OS's, so a 1280x800 board frame is a 1278x798 client area) plus
one JSON record per key. The board loads Inter and JetBrains Mono from Google Fonts, so a capture needs the network.

## Adding a frame

1. **Scene**: add `("<frame-id>", |s| { … })` to your group's `SCENES` (`src/frames/<group>/mod.rs`; `shell::scenes`
   joins the groups' tables: one line per group): what the frame sets on a fresh `State`.
2. **Keys**: open `scripts/keymap.js` (a frame group adds its own file in `scripts/keymap.d/`, read after it). Everything the Files window draws is keyed there already, by the board's
   class names. For what your frame draws in addition, add `api.box` / `api.text` / `api.textNode` / `api.icon`
   lines, by class name, never by position or by string. Key every element a person can see: each icon, each text,
   each box with a fill or a border.
3. **Golden**: `node scripts/capture-golden.mjs <frame-id>` and `--dark`. Commit `golden/<frame-id>*`.
4. **Components**: build the frame from `src/ui/` (below). Give each element its key: `kit::label(key, text,
   style, colour)` for a text and `kit::glyph(key, &ICON, size, colour)` for an icon (the gate reads their font
   and glyph from these, so a bare `text(..)` or `icon(..)` fails the text or icons check as MISSING), `.key(..)`
   for a box. Take measurements from the board's CSS, not from a screenshot.
5. **Tokens and icons**: a new colour, type style or radius goes in `src/theme/tokens.rs` under the board's name; a
   new glyph in `src/theme/icons.rs`, its body copied from the board's inline `<svg>` (the gate compares path data).
6. **Gate**: `scripts/compare.sh <frame-id>` and `--theme dark` until both truth lines show 0 failing four times.
   Read the side-by-side and the tiles map at full size before you call a region done.
7. **Tests**: keyboard, roles and names for what the frame adds (`src/ui/tests.rs` shows how).
8. **PR body**: the two truth lines, before and after.

### The components (`src/ui/`)

| Piece | Function | Keys it gives |
| --- | --- | --- |
| Caption | `caption::caption_bar(t, cx)` | `caption-bar`, `cap-mark`, `cap-title`, `cap-min/max/close` (+`-icon`) |
| Brand block | `sidebar::brand(product, t)` | `brand-mark`, `brand-word`, `drive-label` |
| Sidebar item | `sidebar::side_item(label, icon, count, on, shortcut, t)` | `<label>-row`, `<label>-icon`, `<label>`, `<label>-badge` |
| Toolbar button | `toolbar::tool_button(key, name, icon, size, ToolState, t)` | `<key>`, `<key>-icon` |
| Segmented control | `toolbar::segmented(key, name, vec![segment(..)], t)` | `<key>`, per segment `<key>`, `<key>-icon` |
| Keyboard hint | `toolbar::kbd(key, keys, t)` | `<key>-box`, `<key>` |
| Column headers, rows | `file_list::column_headers(t)`, `file_list::file_row(i, item, selected, t)` | `col-*`, `row-<i>`, `row-<i>-tile/icon/name/lock/date/size/dash/link/avatar-<j>/pc` |
| File tile, thumbnail | `file_list::file_tile(key, item, t)` | `<key>-tile`, `<key>-icon` |
| PC state glyph | `file_list::pc_glyph(key, PcState, surface, t)` | `<key>` (state `online-only`, `downloaded`, `available-offline`) |
| Details panel | `detail_pane::pane(t)`, `preview`, `page_preview`, `encrypted_badge`, `meta_row`, `switch_card`, `switch`, `action_button` | see each function's doc |
| Avatar, mark, shadows | `kit::avatar`, `kit::mark`, `kit::ring`, `kit::ring_inset`, `kit::drop` | the key you pass |
| Status line | `status_bar::status_bar(t)` | `status-bar`, `status-dot`, `status-text`, `status-details` |
| Status line, own words | `status_line::status_line(Option<state>, details, t)` | the same keys |
| Menu | `menu::menu(key, name, width, entries, hot, t, pick, hover, close)`, `menu::Entry` (`.dimmed(off)` for an item that cannot be picked now), `menu::item_key` | `<key>`, `<key>-item-<i>` (+`-label`, `-shortcut`, `-check`), `<key>-sep-<j>` |
| Toast | `toast::layer(toast::toast(icon, tone, message, Option<ToastAction>, t))`; `Tone::Failed` is the red one (DW-0109) | `toast`, `toast-icon`, `toast-text`, `toast-action`, `toast-action-text` |
| Outline button | `button::outline_button(key, label, icon, t)` | `<key>`, `<key>-icon`, `<key>-label` |
| Empty state | `empty::empty_state(key, icon, title, sentence, t)` | `<key>`, `<key>-tile`, `<key>-icon`, `<key>-title`, `<key>-text` |

A hairline is `kit::HAIR` (half a point) in `t.separator`: `.border_sides([..], t.separator).border_layout()` for a
CSS border, `.box_shadow(kit::ring_inset(t.separator))` for a CSS `inset 0 0 0 .5px` ring.

## Structure

```
src/
  main.rs           entry point, State, scenes, fonts
  source.rs         DriveSource trait, BoardFixtureSource (the board's 11 rows), LargeFixtureSource (5k)
  theme/
    tokens.rs       the board's colours (light and dark), type scale, radii, spacing
    icons.rs        the board's SVG icons, and the Beebeeb mark
  frames/
    files/          the Files frames: menus, selection, toasts, folders (FRAMES.md is the checklist)
  ui/
    kit.rs          label / glyph / avatar / mark / shadows, and the notes the gate reads
    gate.rs         --gate-export: the native element tree as JSON
    window.rs       the Files window, assembled
    keys.rs         the Files window's keyboard
    caption.rs  sidebar.rs  toolbar.rs  file_list.rs  detail_pane.rs  status_bar.rs
    tests.rs        keyboard, roles and names
scripts/
  compare.sh        the frame gate, one command
  capture-golden.mjs + keymap.js   the board golden and which element has which key
  gate.py           geometry, pixels, text, icons
golden/
  <frame>[-dark]-board.png, <frame>[-dark].json
```
