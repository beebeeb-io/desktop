# Wave 1: what is open (the start of wave 2)

Written by the integrator (I2) on 2026-11-12 03:30 CET, with the last of the four frame-group pull requests
(#291 sign-in, #292 settings, #293 shared and trash, #294 files). Everything here is copied from those four
pull-request bodies, their `FRAMES.md` checklists and the gate runs on the merged tree; where a line is a lane's
claim that I did not check myself, it says "(lane)". Rune is pinned at `c92e462e`; every `crates/rune/...` path
below is at that revision.

## 1. Frames: built and not built

Of the 185 frames the four checklists list, 55 are built and 130 are not.

| Group | Checklist | Listed | Built | Not built |
| --- | --- | --- | --- | --- |
| Sign-in, gate, first run | `src/frames/signin/FRAMES.md` | 59 | 15 | 44 |
| Settings | `src/frames/settings/FRAMES.md` | 50 | 15 | 35 |
| Shared, Search, Trash | `src/frames/shared/FRAMES.md` | 26 | 13 | 13 |
| Files | `src/frames/files/FRAMES.md` | 50 | 12 | 38 |
| Total | | 185 | 55 | 130 |

Photos, Backups, Sync history, Available offline, the tray flyout and the conflict dialog have no checklist yet,
so they are in neither column.

**Sign-in: built (15)** DW-A01, A02, A03, A04, A05, A06, A07, A08, A09, A10, A18, A19, A20, A21, A22.
**Not built (44):** DW-A11 to A17 (7, approving a sign-in, no goldens); DW-A23 to A30 (8, goldens captured);
DW-A31 to A44 (14, create an account, no goldens, no lane assigned: the lead's call); DW-A45, A46 (goldens
captured); DW-A47 to A50 (4); DW-A63 (golden captured); DW-0009, DW-0014; DW-0380, DW-0301 to DW-0305 (6).
Five controls have no visible effect until those frames exist: `Intent::CreateAccount`, `ForgotPassword`,
`UsePasskey`, `Next(..)`, `AccountDeleted`.

**Settings: built (15)** DW-0701, 0781, 0751, 0782, 0750, 0750b, 0791, 0791b, 0787, 0788, 0788b, 0788c, 0788d,
0789, 0789b. **Not built (35), goldens captured for all:** DW-0708, 0712, 0713, 0714, 0780, 0716, 0716b, 0746,
0746b, 0746c, 0746d, 0746e, 0744, 0744b, 0745, 0785, 0785b to 0785f, 0786, 0786b to 0786d, 0783, 0783b to 0783d,
0784, 0784b to 0784d, 0790, 0790b. Components still needed (lane): `.kit-meter`, `.kit-seg`, `.mk-fig` and its
chart, `.mk-drow`, `.mk-steps`, `.kit-spark`, `.kit-chip`, `.mk-insbar`, `.mk-lrow`, `.mk-skel`, `.mk-sheet`, a
red dialog button.

**Shared and Trash: built (13)** DW-0501, 0502, 0503, 0505, 0506, 0507, 0508, 0509, 0510, 0747, 0748, 0749b,
0117. **Not built (13), goldens captured for all:** the dialogs DW-0504, 0511, 0512, 0512b, 0513, 0513b, 0514,
0515 and DW-0749 (nothing of a dialog exists in this group; DW-0749's code boxes are secret-bearing); Search
DW-0801, 0802, 0802b, 0803 (`src/frames/search/` does not exist).

**Files: built (12)** DW-0401, 0402, 0404, 0405, 0406, 0414, 1101, 1104, 1106, 0107, 0108, 0109.
**Not built (38):** DW-0403, 0407, 0408, 0408b, 0409, 0410, 0410b, 0411, 0412, 0413, 0415, 0416, 0417, 0480,
0481, 0420, 0420b, 0421 to 0427, 0429, DW-1102, 1103, 1105, 1107 to 1110, DW-0101, 0102, 0114, 0118, 0119, 0121.
Goldens exist for DW-0403, 0410, 0410b, 0412, 0417, 0420, captured BEFORE their new keys were written.

## 2. Gate lines that are not at 0 failing

Gate run on the merged tree of #294 (110 lines: 55 frames, light and dark): 91 lines at 0 failing on all six
counts, 19 with failing pixel checks. Geometry, text, icons and fills are 0 failing on all 110. No tolerance was
raised and no key dropped for any of them.

| Frame | Theme | Failing | Element | Cause as far as known |
| --- | --- | --- | --- | --- |
| DW-A04 | light, dark | pixels 29 / 26, frame tiles 30293 / 2159 | most of the frame | the board golden holds Windows' own Windows Hello dialog and scrim. **Ruling (lead, on #291): recapture the golden with that layer hidden.** Not done yet |
| DW-A05 | light, dark | pixels 33 / 30, frame tiles 30235 / 2634 | most of the frame | the same |
| DW-A07 | light, dark | pixels 2 | `status-ring` (edge 22 > 9), `status-dot` (edge 10 > 9) | the 26 pt ring is one device pixel further left; the 6 pt dot is the small-disc rim below |
| DW-A19, A20, A21 | dark | pixels 1 | `card-0-avatar` (edge 10 > 9) | not solved; same colours measured on both sides, light passes |
| DW-A22 | light, dark | pixels 3 / 4, frame tiles 67 / 66 | `banner-0`, `banner-0-text`, `au-main`; dark also `card-0-avatar` | the note wraps after "to 15" natively, after "to" on the board (12.5 px Inter in 292 px) |
| DW-0791 | light, dark | pixels 2 | `pill-0-dot`, `pill-1-dot` (edge 11 / 10 > 9) | small-disc rim |
| DW-0787 | dark | pixels 1 | `item-0-0-pop` (edge 14 > 9) | a keyed box under an open menu; its edge runs through the menu's text (two text rasterisers) |
| DW-0789b | dark | pixels 1 | `item-1-1` (edge 10 > 9) | the same, under the dialog |
| DW-0502, DW-0506 | light, dark | pixels 1 | `item-0-2-pill-dot` (edge 11 / 10 > 9) | small-disc rim |

Three causes cover 13 of the 19 lines: the small-disc rim (DW-A07's dot, DW-0791, DW-0502, DW-0506: 9 lines), the
Windows Hello layer (4 lines), an edge check through overlaid text (2 lines). They want ONE decision each, not a
fix per frame.

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
  crate's own code is at 0. The "Frontend (bun test + tsc + eslint)" check is red on `v2-native` (no React work
  for 0.9.0).

## 8. What nobody did in wave 1

No frame was looked at in a real Windows window: every picture is a headless render (lavapipe) against the board,
and the Windows build VM ran `cargo test`, not the frame gate. No screen reader, no real Tab order in a real
window, no real drop from File Explorer, no Windows Hello prompt. `cargo test --features feedback` was run by
the settings lane only (224 passed, before its last merge), never on the merged tree.
