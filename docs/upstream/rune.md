# Rune: issues we work around

Rune (`anomalyco/rune`, pinned at `c92e462e` in `native/beebeeb-drive-rune/Cargo.toml`) takes issues, not pull
requests. Filing them is the lead's step. Each entry: what is wrong, the minimal repro, where we work around it, and
what we delete when it is fixed. Entry 1 is measured in this repo's tests; for 2 to 5 the repro is the smallest
program we can describe, and it must be run against a Rune checkout before filing (marked *to confirm*).

## 1. Text events: `VK_PACKET` and dead-key characters are lost (Windows)

Rune builds a key event from the logical key only. winit reports a `VK_PACKET` press (RustDesk, an RDP client's
Unicode mode, an on-screen keyboard, `SendInput` with `KEYEVENTF_UNICODE`) as an unidentified key with the character
only in the event's text, so Rune drops the character. The key after a dead key has the same loss: with US-International,
`'` then Space is the Space key with the text `'`, and `'` then `t` is the T key with the text `'t`; Rune types a space
and a `t`, so an apostrophe or a quote cannot be typed.

Repro: a window with one `text_input`; on Windows send `SendInput` with `KEYEVENTF_UNICODE` for `é`, or set the
US-International layout and type `'` then Space. Expected: `é` / `'`. Got: nothing / a space.
Fix wanted: a key event keeps the text of winit's event when the logical key does not carry it.
Workaround: `native/beebeeb-drive-rune/src/shell/platform/packet_text.rs` (the rule) and `windows_packet.rs` (a
`WH_GETMESSAGE` hook that takes the `WM_CHAR` and hands the characters to the focused field). Delete both on the fix.

## 2. Focus: no way to ask which element holds the keyboard, and no focus-change callback for elements

A view cannot ask which element has the keyboard (`App::on_focus` only reports the window). A headless render has no
focus at all. Repro: build a `col` of two `text_input`s, try to draw a ring on the focused one from the view function:
there is no query. Workaround: every field has `focus_within_ring` plus a `pictured` flag for scenes
(`src/ui/au_entry.rs`); roving Tab stops are managed by hand (`src/ui/focus.rs`). Fix wanted: an element-level
focus-changed callback, or a `focused(key)` query.

## 3. Windows clipboard: one format only (*to confirm*)

Copying from the app puts one text format on the clipboard. File lists (`CF_HDROP`), rich text and a private format for
"copy files in the vault" cannot be offered together with text, so Paste in File Explorer or another app does not
see the files. Repro: copy from a `text_input` and from a file row; inspect with a clipboard viewer: one format.
Fix wanted: a clipboard API that sets several formats in one go (text plus `CF_HDROP` plus custom).

## 4. Drag out of the window (*to confirm*)

Drops into the window arrive (`DW-0481`, upload by dropping files); there is no way to start a drag from an element,
so a file row cannot be dragged to File Explorer or the desktop. Repro: any element with a pointer-down handler; no
call starts an OS drag with file paths. Fix wanted: `start_drag(paths)` on a pointer event (Windows `DoDragDrop` with
`CF_HDROP`). Also: the hover event during a drag carries no item count (`src/frames/files/FRAMES.md`, DW-0481).

## 5. Soft-wrap and tab-stop API for text editing (*to confirm*)

The text viewer and the office editor need the layout of a wrapped line (where a visual line breaks, caret position on
a wrapped line, Home/End per visual line) and a configurable tab-stop width. Rune's text input exposes neither (`src/text_kit/` works on byte offsets of logical lines only, so it cannot
supply them). Repro: a multi-line `text_input` with a long line
and a tab character: no API gives the visual line ranges or sets the tab width. Fix wanted: read-only layout
(`visual_lines()`), and a `tab_width` style property.

## Smaller notes (not issues yet)

- The field submit event carries no modifiers (Shift+Enter in find cannot step back: `src/frames/viewers/FRAMES.md`).
- A focused field gets its keys before any ancestor (`src/frames/viewers/new_file.rs`).
- The viewport has no scroll bar of its own (`src/frames/office/docs_scroll.rs`); the glyph atlas has no masks.
