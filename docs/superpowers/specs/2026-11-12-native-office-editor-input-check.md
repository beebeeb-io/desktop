# Office editor, PR 0: the Rune input check

Result of row 0 of the plan in `2026-11-12-native-office-editor.md` (§7), which named this as risk 1: "Rune's
support for a custom editing surface with an input method is unverified".

**Verdict: the editor's interaction layer CAN be built on Rune as pinned.** 5 of 5 points are yes on Windows, in
a real window, with a real Japanese input method. There is no "no". There are 7 limits (§4); none stops the
editor, 2 are worth an upstream issue.

Not covered: macOS and Linux windows (nothing was run in a window there), input methods other than the Microsoft
Japanese one, a display scale other than 100 % in a real window, touch and pen, accessibility of the surface.

## 1. What was built

`spikes/2477-rune-input-check/`: a standalone crate (not a workspace member, not built by CI) with one dependency,
the native app's own Rune line (`rev = "c92e462e"`, `default-features = false`, `features = ["accessibility"]`).
Its lock started as a copy of the native app's: `rune`, `winit` 0.30.13, `arboard` 3.7.0, `wgpu` 30.0.1 and
`parley` 0.12.0 resolve to the same versions and sources in both locks (5 of 5 compared).

The probe is a canvas-like surface with no Rune text field in it: a plain focusable element whose characters,
selection, composition underline and caret are elements the app places itself, one fixed-width cell per character.
It types, moves and selects, shows the pre-edit string underlined at the caret, commits it, copies, cuts and pastes.

| File | What it is |
|---|---|
| `src/lib.rs` | the model, the view and the handlers |
| `src/main.rs` | the same app in a real window; writes every event it hears to a log; on Windows reads back the window's input-context forms |
| `tests/input.rs` | 15 tests through Rune's headless `Runtime` |
| `mutants.py` | 15 mutants, each removing one piece of Rune wiring; counts which tests go red |
| `win/run.ps1` | drives the real window with OS input (`SendInput`) and checks the probe's log: 28 checks |
| `win/ime-japanese*.ps1` | adds or removes Japanese in the test account's language list |

## 2. Result per point

| # | Point | Result | Headless (Linux and Windows) | Real window (Windows) |
|---|---|---|---|---|
| 1 | Key presses and committed text | **yes** | 2 tests | 5 checks |
| 2 | Composition: start, update with pre-edit and caret, commit, cancel | **yes** | 4 tests | 7 checks, Microsoft Japanese IME |
| 3 | Candidate-window position | **yes** | 3 tests | 2 checks on the input context (6 of 6 caret positions), 1 screenshot looked at |
| 4 | Clipboard | **yes**: text and HTML both ways; limits in §4 | 2 tests | 4 checks against another process |
| 5 | Focus, pointer press, drag, release | **yes**; focus LOSS has no callback (§4.1) | 4 tests | 9 checks |

Counts: headless 15 of 15 pass on the Linux and on the Windows build machine; mutants 15 applied, 15 killed, 15 of
15 tests red on at least one; real window 28 checks, 28 passed, 0 failing, 0 skipped.

What each tier proves. Headless: Rune delivers the event to an app-drawn surface (the tests only feed the
`Runtime`; no probe handler is called directly). It proves nothing about Win32. Real window: the same through
Windows' own keyboard, mouse, input-method and clipboard paths, observed from outside the process where possible
(the clipboard is read and written by another process; the input-context forms are read back with
`ImmGetCandidateWindow` and `ImmGetCompositionWindow`).

## 3. The Rune API, per point

Paths are in `crates/rune/src` of the pinned revision.

**1. Keys and text.** `El::focusable(true)` and `El::on_key` (`el/focus.rs`) on the canvas element. Typed
characters arrive as `Key::Text`, shortcuts as `Key::Command(c)` with `KeyEvent::mods` (`edit/keys.rs`); releases
through `El::on_key_up` (`input/canvas_input.rs`). The handler runs before focus navigation, so the editor decides
about arrows, Home, End and Tab itself. Plain typing is never an input-method commit: each character arrives once.

**2. Composition.** `App::on_ime` (`app.rs`; `WindowSpec::on_ime` for one window of several) receives `Ime`
(`edit/ime.rs`): `Preedit(text, Option<(start, end)>)`, `Commit(text)`, `Enabled`, `Disabled`. Dispatch is in
`runtime/input/app_ime.rs`. There is no separate start or cancel event:

- start is the first non-empty `Preedit`;
- the range is in BYTES of the pre-edit string; while typing it is a caret (`(6, 6)`), while converting it is the
  clause being converted (`(0, 6)`);
- a commit arrives as `Preedit("")`, then `Commit(text)`, then `Disabled`;
- a cancel (Escape) arrived as `Preedit("")`, `Preedit("")`, `Commit("")`, `Disabled`: an EMPTY commit. An editor
  must insert nothing for it. This sequence is now a headless test.
- `Enabled` and `Disabled` bracket each composition on Windows; they do not mean "the input method was switched".

While the app's composition is open, Return and Escape go to the input method and not to `on_key` (Rune), and on
Windows Backspace and the letters did not reach `on_key` either (0 key events in 3 checks).

**3. Candidate position.** `El::ime_cursor()` (`el/layout.rs`) on the caret element the app draws. While that
element is drawn and no Rune text field has focus, Rune turns the window's input method on and gives the platform
the element's drawn rectangle every frame (`window/platform_sync.rs`: `set_ime_allowed`,
`set_ime_cursor_area`). Headless it is the public field `Runtime::ime_area`. On Windows the input context held
the caret rectangle on 6 of 6 positions checked (candidate form: exclude-rectangle style at the caret; composition
form: the point under the caret), it followed the pre-edit cursor during a real composition, and the candidate list
opened directly under the pre-edit. The rectangle is in points at any scale (headless, scale 1 and 2).

**4. Clipboard.** `Tx::copy`, `Tx::copy_rich(ClipboardContent)`, `Tx::read_clipboard(callback)`,
`Tx::request_paste` (`clipboard/tx.rs`); `ClipboardContent` (`clipboard/content.rs`) carries text, HTML, RTF, an
image and a file list; a root element can take pastes with `El::on_paste` / `on_paste_content`
(`el/handlers.rs`). The probe copies in `on_key` on `Key::Command('c')` and reads in `on_key` on
`Key::Command('v')`, which needs no handler on the window's root. Tests use `rune::clipboard::memory()` with
`Runtime::use_clipboard`. On Windows (`clipboard/native.rs`): another process read the copied text and its
`HTML Format`; text and text-with-HTML put there by another process were read back with both forms.

**5. Focus and pointer.** A press on a focusable element focuses it; `Tx::focus(key)` and `Tx::blur()` move focus
from code. `El::on_drag` (`el/handlers.rs`) gives `Phase::Down`, `Move`, `Up`, the position in the element's OWN
points (also under a scale or scroll), `Drag::clicks` for double and triple clicks and `Drag::cancelled`;
`Tx::mods()` gives Shift for a press; `El::pointer_buttons` adds the middle and side buttons; `El::on_wheel` and
`El::on_magnify` take scroll and zoom before any scroll view (`input/canvas_input.rs`). The window's own focus:
`App::on_focus`.

## 4. Limits found

| # | Limit | What the editor does | Upstream issue? |
|---|---|---|---|
| 1 | **No per-element focus callback.** An app-drawn focusable element is not told when it gains or loses keyboard focus. Only `Tx::focused()` says who has it, inside a handler or `AppSpec::poll` | Reads `Tx::focused()` in `poll` (runs at the start of each frame) and keeps "has the keyboard" in its own state. Without a frame after the change, the caret would be stale: the real-window probe wakes itself every 50 ms for it | **Yes.** "A focusable element that draws its own caret needs a focus-change callback (gained, lost), as text fields have internally" |
| 2 | **Key, input-method and press commits animate layout by default.** A caret moved in `on_key` glides, and the candidate rectangle glides with it (3 tests were red on it; one mutant puts 2 of them back to red) | Every handler calls `Tx::instant()` | No: documented behaviour, one line per handler |
| 3 | **`El::on_drawn_layout` is not a reliable position report after instant moves in a real window:** it reported 16 of 29 caret positions in one run, while the input context was right on every one. Headless it reported all | Takes the caret rectangle from its own model, never from this callback | **Maybe.** Cause not established (could be the probe's use of it); worth a minimal reproduction before filing |
| 4 | **The pre-edit has no clause styling.** One byte range comes with it; which clause is converted and which is plain is not distinguishable beyond that range (the window layer passes no attributes) | Underlines the whole pre-edit and marks the given range | No for 0.9.0; a known limit of the window layer |
| 5 | **Clipboard on Windows writes ONE form per copy:** files, else an image, else HTML beside the text, else text. RTF is neither written nor read on Windows. There is no custom format | Formatted copy and paste between Office apps goes through HTML. Lossless copy between two of our own editor windows cannot ride the clipboard in a private format; it needs HTML that round-trips or an in-app side channel | **Yes**, if we want it: "write several forms at once on Windows, and a named custom format". Not needed for plain text and HTML |
| 6 | **`App::on_ime` is one function pointer per window**, and `El::on_paste` is root-only | PR 8 adds one `on_ime` line to the shell's window spec and routes to the open editor; pastes are read in the editor's own `on_key`, which needs no root handler | No |
| 7 | **Only the first `ime_cursor` element counts, and a focused Rune text field wins** (tested) | One caret per window; a cell editor or search field built from a Rune text field takes the input method by itself | No |

Also observed, not a limit: Rune intercepts Ctrl+V itself only when the window's root has a paste handler;
without one the chord reaches `on_key`, which is what the probe uses.

## 5. Evidence and how to repeat it

```sh
# headless tests and mutants (any machine with the toolchain; the mutants rewrite src/lib.rs and restore it)
cargo test --locked
python3 mutants.py        # last line: mutants: 15 applied, 15 killed; tests: 15, 15 red on at least one
# real window, inside a logged-in Windows desktop session with BB_OUT and CARGO_TARGET_DIR set
cargo build --locked
powershell -NoProfile -File win\run.ps1     # last line: real-window: 28 checks, 28 passed, 0 failing, 0 skipped
```

The composition leg of `win/run.ps1` needs a Japanese input method for the account; without one its checks are
SKIP, never PASS (seen: 19 passed, 6 skipped before the input method was there). The Microsoft Japanese IME needs
the Windows capability `Language.Basic~~~ja-JP~0.0.1.0`; adding the language to the account's list alone leaves
it "not ready" and typed letters arrive as plain keys.

Three instrument faults were found and fixed on the way, each first read as a product failure:

1. PowerShell names ignore case, so a parameter `$Cell` and a constant `$CELL` were one variable and the pointer
   went to `cell * cell`. Renamed.
2. The window was looked up through the process's main window, which stopped being the probe's once an input method
   was installed; clicks then landed on the desktop. The script now finds the window by its title and stops when
   there is none.
3. The script compared the input context with the rectangle `on_drawn_layout` reported, which lags (limit 3). It
   now compares with the model.

## 6. For PR 7 and PR 8

- The caret element carries `ime_cursor()` only while the editor has the keyboard; its rectangle comes from the
  layout crates' `caret` functions (spec §4.1), converted to the canvas's points.
- Every editor handler says `Tx::instant()`.
- Treat `Commit("")` as nothing, `Preedit("")` as "composition ended", and the `Preedit` range as bytes.
- Keep "has the keyboard" in editor state, set from `Tx::focused()`; do not wait for a callback.
- Copy as text plus HTML; read with `Tx::read_clipboard` and take HTML when it is there.
- The helper process (spec §5) sees none of this: composition stays in the UI process until `Commit`, so a
  keystroke of pre-edit costs no round trip.
