# Sign-in, gate and first run: the frame checklist (lane f-signin)

The Windows frames of the board's groups First run, Sign in, Continue as, Another device, Approving a sign-in,
Email and password, Two-factor and passkeys, Forgot password, Before Files, and The first-time guide: 45 frames;
and Create an account (DW-A31 to DW-A44): 14 more Windows frames, which the board reaches from DW-A01, DW-A02 and
DW-A03 ("Starts sign-up at the email step DW-A31"). 59 frames in all.

Built = drawn from `Ui`, reachable by its controls, a scene in `shell/mod.rs`, gated in light and dark.
Gate state per frame is in the PR body; `0` = all six counts at 0 failing in both themes.

| Frame | What | State |
| --- | --- | --- |
| DW-A01 | First screen: create an account, or sign in | built, 0 |
| DW-A02 | Sign in, with the Continue as card | built, 0 |
| DW-A03 | Sign in, no other Beebeeb app signed in | built, 0 |
| DW-A04 | Continue as: Windows Hello is asking | built; pixels differ: the board draws Windows' own dialog over the frame |
| DW-A05 | Continue as: not recognized | built; pixels differ: the board draws Windows' own dialog over the frame |
| DW-A06 | Continue as: cancelled | built, 0 |
| DW-A07 | Another device: QR code, code, countdown | built; 2 pixel lines open (`status-ring`, `status-dot`) |
| DW-A08 | Another device: approved | built, 0 |
| DW-A09 | Another device: denied | built, 0 |
| DW-A10 | Another device: the code expired | built, 0 |
| DW-A11 to DW-A17 | Approving a sign-in (7 frames) | built in the account view (`.au-set`), where the board draws them: `frames/account/approve.rs`, rows in `frames/account/FRAMES.md` |
| DW-A18 | Sign in: unlocking the vault | built, 0 |
| DW-A19 | Sign in: email and password do not match | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A20 | Sign in: offline | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A21 | Sign in: too many tries from this network | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A22 | Sign in: the lock explained | built; the note wraps one word later than the board (3 pixel lines), dark also `card-0-avatar` |
| DW-A31 | Create an account, step 1: the email | built; tree gate 0, pixels owed (PR body) |
| DW-A32 | Step 2: the 8-digit code from the email, four digits typed | built; tree gate 0, pixels owed (PR body) |
| DW-A33 | Step 2: that code is not right, or it has expired | built; tree gate 0, pixels owed (PR body) |
| DW-A34 | Step 2: a new code was sent | built; tree gate 0, pixels owed (PR body) |
| DW-A35 | Step 3: the terms, both boxes ticked | built; tree gate 0, pixels owed (PR body) |
| DW-A36 | Step 4: the password, strong | built; tree gate 0, pixels owed (PR body) |
| DW-A37 | Step 4: the password is two characters short | built; tree gate 0, pixels owed (PR body) |
| DW-A38 | Step 4: the password is known from a breach | built; tree gate 0, pixels owed (PR body) |
| DW-A39 | Step 5: the recovery phrase, shown once | built; tree gate 0, pixels owed (PR body) |
| DW-A40 | Step 5: three words typed back, with suggestions | built; tree gate 0, pixels owed (PR body) |
| DW-A41 | Step 5: the words do not match | built; tree gate 0, pixels owed (PR body) |
| DW-A42 | Step 6: this device's name | built; tree gate 0, pixels owed (PR body) |
| DW-A43 | Creating your account | built; tree gate 0, pixels owed (PR body) |
| DW-A44 | Your account is ready | built; tree gate 0, pixels owed (PR body) |
| DW-A63 | The account was deleted: the sign-in form says so under the password | built; light 0, dark 1 pixel line open (`card-0-avatar`, as DW-A19) |
| DW-A23 | The 12 words on a new device (five typed, the sixth box holding the keyboard) | built, 0 |
| DW-A24 | The words did not open this account (the third box marked) | built, 0 |
| DW-A25 | No phrase and no signed-in device: the three exits | built, 0 |
| DW-A26 | Two-factor: six digit boxes, the fifth next | built, 0 |
| DW-A27 | Two-factor: that code isn't right | built, 0 |
| DW-A28 | Two-factor: a backup code | built; light 0, dark 1 pixel line open (`field-0-caret`: the board's caret starts on half a device pixel) |
| DW-A29 | A passkey: Windows Security is asking | built; pixels differ: the board draws Windows' own dialog over the frame (as DW-A04) |
| DW-A30 | A passkey, then the vault password | built, 0 |
| DW-A45 | Trouble signing in? Three honest options | built, 0 |
| DW-A46 | We cannot recover your account | built, 0 |
| DW-A47 | Recovery: the email and the twelve words (eight typed) | built, 0 |
| DW-A48 | Recovery: the phrase did not verify | built, 0 |
| DW-A49 | Recovery: a new password, with the strength meter | built, 0 |
| DW-A50 | Password updated | built, 0 |
| DW-0009 | Beebeeb is locked (the main window, gate `Locked`) | built (`before_files.rs`); geometry, text, icons and fills 0 failing in both themes; pixels NOT measured: `--snap` of a gate scene draws the Files window (the regression on `v2-native`), and the board draws Windows Hello's prompt over the frame |
| DW-0014 | No plan yet (the main window, gate `NoPlan`) | built (`before_files.rs`); geometry, text, icons and fills 0 failing in both themes; pixels NOT measured: the same `--snap` regression |
| DW-0380, DW-0301 to DW-0305 | The first-time guide | see "The first-time guide" below |

Built: 52 of 59 here (DW-0009 and DW-0014 in `before_files.rs`, DW-A31 to A44 in `create.rs` and its files, the 6 of
the first-time guide; the count is the lanes' lists joined at their merge), and the 11 frames of the first run below:
63 of 70.
Not built in this group's files: 7 (DW-A11 to A17); the 6 frames of the first-time guide are below (lane b04).
Since then DW-A12 to DW-A17 are built in the account view (the row above; DW-A11 is Windows' toast).

## First run on Windows (lane f-signin-rest): `setup/`

The board's group "First run on Windows", 11 frames, in the same 960 x 640 window: `setup/mod.rs` (state, scenes),
`setup/view.rs`, `setup/source.rs` (`SetupSource` and the board's fixture), `ui/setup.rs` (the window and its controls).
`0` = all six gate counts at 0 failing in both themes.

| Frame | What | State |
| --- | --- | --- |
| DW-0201 | Step 1: add a passkey | built, 0 |
| DW-0202 | Step 2: app lock, lock again after | built, 0 |
| DW-0203 | Step 3: back up the six known folders | built, 0 |
| DW-0204 | Step 4: thumbnails and previews (sample, size, format, space) | built, 0 |
| DW-0209 | The compare view over step 4 (700 x 520 sheet) | built; light 0, dark 1 pixel line open (`cmp-opt-5 tile 27 > 26`) |
| DW-0206 | Step 5: Beebeeb is in File Explorer | built; 1 pixel line open in each theme (`nav-2-b`, the b's plate) |
| DW-0206b | Step 5: still adding | built, 0 |
| DW-0280 | Step 5: Windows did not add it | built, 0 |
| DW-0207 | Step 6: notifications | built, 0 |
| DW-0281 | Step 7: open at login | built, 0 |
| DW-0208 | You're set up | built, 0 |

## The first-time guide (lane b04): `setup/guide.rs`

A coach card over the main window with a ring around the real control each tip explains (`cx.window_layout` of the
control's key: the ring follows it). State: `State::guide`. It starts when the first run ends ("Open my Beebeeb",
`guide::poll`) and from Settings › General › "Show the guide again" (`settings::go("DW-0380")`). Got it, Next and Done
step; Skip and Esc end it; the window under a tip takes every press, and whatever the section has in front (a menu,
a dialog, the New file dialog, an opened photo) hides the tip and gets Esc first. The keyboard is the card's while a tip shows (focus on
its button, Tab stays in the card, Enter presses). It ends when the account locks, signs out or changes. "Once per
PC" is `SetupSource::guide_shown` / `mark_guide_shown` (not wired: the guide follows each first run). Tests:
`setup/guide_tests.rs`. Not built: the second way back the board names ("and Help"); the card's 22 pt button is
drawn here, not by the kit (a design-system need).

| Frame | What | State |
| --- | --- | --- |
| DW-0380 | Where the b is: "Beebeeb lives in the notification area", Got it | built in the main window (its lower right corner, clear of the details panel's buttons). Red on every dimension but text, icons and fills (225 of 225 geometry keys): the board composes a desktop with the window at 0.72 scale at (18, 18), the taskbar and the card by the tray; the app draws its window at full size. Follow-up task 2479 (the card in a window of its own by the notification area) |
| DW-0301 | Tip 1: New | built; geometry, text, icons, fills, frame tiles 0; 3 pixel keys open: `coach-ring` (New's top edge at 40.75 pt: the board paints it from device row 81, Rune from 82, a 1 px step inside the ring), `coach-dot-0`, `coach-dots` |
| DW-0302 | Tip 2: a file's row (left out when the folder shows no file) | built; 1 pixel key open (`coach-dot-1`) |
| DW-0303 | Tip 3: Shared, on the Shared view | built; 2 pixel keys open: `coach-dot-2`, and `card-0-tile` (under the card: the card stands 0.54 low with the sidebar's rows, and in dark a glyph row of the tip's words differs) |
| DW-0304 | Tip 4: Photos, on the Photos view | built; 1 pixel key open (`coach-dot-3`) |
| DW-0305 | Tip 5: the gear | built; 2 pixel keys open (`coach-dot-4`, `coach-dots`) |

Measured on the Linux build VM, light and dark the same keys (2026-11-13). The open `coach-dot-*` keys (and
`coach-dots`, whose end is the lit dot) are the 6 pt dot's anti-aliased edge: the board's edge pixels are lighter
(86 / 121 / 186 against Rune's 74 / 107 / 169 at 2x), as `status-dot` in DW-A07; Rune's drawing, not this group's. The
sidebar's rows stand 0.5 pt low in every Files-window frame (`drive-label` is 12 high, the board's 11.5: `ui/sidebar.rs`),
and the ring and card of tips 3 and 4 follow them. The words of DW-0380 are the board drawing's (`guide::TRAY_TEXT`); the
frame's note in `frames.json` has other words (decision 2478).

States with a rule and no drawing, built from the rule's words (tests only): the compare view's Stacked and Hold to
flip; "You're set up" after Not now or Skip setup ("Not added", "Off"); Turn on backup with no folder on (disabled).

States the board gives a rule and no drawing, built from the rule's own words (no golden, tests only):
DW-A07 with the pill reading "Reconnecting…"; DW-A08's error state "Approved, but the keys did not arrive.
Nothing was signed in. Ask again."; DW-A21's wait counting down in the message; a code being checked (the digits
dimmed, "Verifying…"); a wrong backup code; a code, a phrase or a vault password while offline or after too many
tries (the sign-in form's words for the same problem; "Too many tries. Wait an hour."; "Too many attempts. For
security, wait an hour and try again."); a vault password or a new password the engine refuses.

## Where each screen lives

| Screens | State and rules | View |
| --- | --- | --- |
| DW-A01 to A10, A18 to A22, A29, A63 | `mod.rs` | `view.rs` |
| DW-A23, A24, A26 to A28, A30 | `steps.rs` | `view_steps.rs` |
| DW-A25, A45, A46 | `steps.rs` | `view_recovery.rs` |
| DW-A47 to A50 | `steps.rs` | `view_recover.rs` |
| DW-A31 to A38 | `create.rs`, `create_source.rs` | `view_create.rs`, `create_kit.rs` |
| DW-A39 to A44 | `create.rs`, `create_source.rs` | `view_create_phrase.rs`, `create_kit.rs` |

`ui/au_entry.rs` has the digit boxes, the word boxes, a field drawn holding the keyboard, and links `au::link` does
not draw. Scenes of DW-A23 and later are `signin::FRAMES` (`frames/mod.rs`), not lines in `shell/mod.rs`.

## For whoever builds the rest

- Keys for every `.au-mac` frame are already in `scripts/keymap-signin.js` (digits, words, cards, lists, strength bars).
- Sign-up's pieces (the progress line, a checkbox, a chip, the meter with sign-up's states, a field that is focused
  and in error at once) are in `create_kit.rs`. The strength meter of recovery is `view_recover.rs::strength`.
- A secret that the screen shows as typed (a code, a recovery word, a backup code) is a secure `TextEdit` made ONCE
  in `Ui::new`, revealed, or a secure text node (`au_entry::shown_secret`); it is never an element label, never
  measured with `rune::text::advance`, never copied into a `String`. `steps_tests.rs`:
  `a_typed_code_and_typed_words_are_in_no_tree_and_no_label` is the pattern to copy.
- A headless render has no keyboard focus: a frame the board draws with a focused field names it in
  `steps::Pictured` (scenes only; the first key or press clears it).
- An engine answer is applied only by the wait it answers (`Ui::apply`, `Ui::apply_step`: every arm has a guard); a
  new wait gets a guard and a line in `back_from_the_code_discards_the_partial_sign_in_and_a_late_answer_changes_nothing`.

## Create an account (lane signin-create): DW-A31 to DW-A44

Sign-up is a set of screens of this window (`Screen::Create(Step)`). "Create a Beebeeb account" (DW-A01) and
"Create an account" (DW-A02, DW-A03) still emit `Intent::CreateAccount`; the shell's poll answers it with
`Ui::start_create`, which opens DW-A31 when the source has sign-up (`SignInSource::sign_up_available`) and
changes nothing when it has not (the engine-backed source, until phase 2). A build that ships opens it on
no fixture: `start()` sets the fixture's `available` to `DEV_BUILD`.

Secret-bearing screens: DW-A36, DW-A37, DW-A38 (the password, typed twice), DW-A39 (the recovery phrase,
shown), DW-A40, DW-A41 (three of its words, typed, and the suggestions that complete them). All are screens
of `FrameGroup::SignIn`, which is excluded from capture as a whole (`shell/host_tests.rs`:
`sign_up_opens_in_the_sign_in_window_and_every_screen_of_it_stays_excluded_from_capture`).

- The password is in `Ui::password` and `Ui::confirm` (the group's secure fields) and is cleared when the
  breach check passes: the engine keeps it. The phrase is never in this window's state: the view borrows it
  for one draw (`with_sign_up_phrase`) and draws each word as a secure text node.
- Decision (lane, 2026-11-12): the twelve shown words and the suggestion chips are secure text nodes with no
  accessibility text, because the group's rule is that a secret is never an element label. The board asks
  that "words are read in order with their numbers" (DW-A39, Accessibility): the two rules contradict, and
  a screen reader reads "Word 1" to "Word 12" without the words until the owner rules on it.
- Decision (lane, 2026-11-12): Copy builds the one `String` of the phrase at its final size and moves it to
  Rune's secret clipboard (zeroizing, kept out of clipboard histories), cleared after 60 seconds or when
  sign-up ends. Save as file and Print are `Intent`s that carry no word; the shell does not act on them yet.
- Decision (lane, 2026-11-12): Back from the terms goes to the email step, as "Use a different email" does,
  because the board names no target and a verified code is spent.
- Decision (lane, 2026-11-12): "the other apps can continue as Anna" takes the name from the part of the
  address before the at sign, because a new account has no name of its own yet.
- The breach check runs on Continue only; the board also asks for it when the field loses focus.

States with a rule and no drawing, built from the rule's words (tests only): the email's three messages
(not an address, too many requests, could not send); "Too many wrong codes. Ask for a new one."; "Send a
new code" in place of the countdown; a breach check that cannot be reached; "Does not match yet."; DW-A43's
three failures (nothing stored with Try again, created but not finished with Sign in only, the code expired
with the way back to Verify). Words that are NOT the board's: "We could not check the code. Check your
connection and try again." (a code checked while offline), and the two headings of DW-A43's failures.
