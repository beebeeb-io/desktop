# Beebeeb Drive — Rune native UI

Native Windows desktop app for Beebeeb Drive, built with [Rune](https://github.com/anomalyco/rune).

## Build

```sh
# Source lavapipe for headless rendering on WSL
source ~/code/bb-legion-native/lavapipe/env.sh

# Shared target dir (warm cache)
export CARGO_TARGET_DIR=~/code/bb-worktrees/target-rune-spike

# Run (opens window on Windows, headless on WSL)
cargo run -p beebeeb-drive-rune

# Headless snapshot
cargo run -p beebeeb-drive-rune -- --snap DW-0401.png --size 1280x800 --scale 2 --theme light

# Tests
cargo test -p beebeeb-drive-rune
```

## Adding a frame

1. **Golden**: capture the board frame with Playwright at 2x into `golden/<frame-id>-board.png`.
2. **Tokens**: if the frame introduces new colours or spacing, add them to `src/theme/tokens.rs` (or regenerate with `python3 scripts/gen-tokens.py`).
3. **Icons**: extract new SVG icons from the board into `src/theme/icons.rs` — each is a `&'static Icon` with an SVG body.
4. **Components**: add or modify components in `src/ui/`. Each component takes `&State` and `&Theme`, returns `El`.
5. **Fixture**: add fixture data in `src/source.rs` matching the board's exact items.
6. **Scene**: add a scene entry in `main.rs`'s `AppSpec::scenes` that sets up state for the frame.
7. **Compare**: run `scripts/compare.sh <frame-id>` — it renders headless and reports pixel diff%.
8. **Threshold**: optionally set `golden/<frame-id>-threshold.txt` (one number, default 15.0).
9. **Tests**: add `cargo test` assertions for the frame's elements.

## Structure

```
src/
  main.rs           entry point, State, view assembly, key handling, tests
  source.rs         DriveSource trait, BoardFixtureSource (11 items), LargeFixtureSource (5k)
  theme/
    tokens.rs       design tokens from the board CSS (colours, spacing, radii, type scale)
    icons.rs        29 SVG icons from the board's DW-0401 frame
  ui/
    caption.rs      caption bar: b mark, DRIVE label, window drag
    sidebar.rs      sections with icons/badges, THIS PC, storage meter, account
    toolbar.rs      nav, title, view toggle, sort, panel toggle, + New, search
    file_list.rs    column headers, virtual-scrolled rows with type icons
    detail_pane.rs  preview, metadata, encrypted badge, action buttons
    status_bar.rs   sync status with green dot
scripts/
  compare.sh        render + compare against golden
  compare.py        Pillow pixel diff with per-region breakdown
  gen-tokens.py     extract board CSS into tokens.rs
golden/
  DW-0401-board.png golden screenshot from the board (2560x1600, 2x)
```
