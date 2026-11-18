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
| DW-0009 | Beebeeb is locked (the main window, gate `Locked`) | built (`before_files.rs`); geometry, text, icons and fills 0 failing in both themes; pixels NOT measured (measured 2026-11-13 in the integration loop: not at 0): `--snap` of a gate scene draws the Files window (the regression on `v2-native`), and the board draws Windows Hello's prompt over the frame |
| DW-0014 | No plan yet (the main window, gate `NoPlan`) | built (`before_files.rs`); geometry, text, icons and fills 0 failing in both themes; pixels NOT measured (measured 2026-11-13 in the integration loop: not at 0): the same `--snap` regression |
| (no board id) | The credential store did not answer (the main window, gate `KeyStoreUnavailable`): Try again, Get help, Quit | built (`keystore.rs`, w16-gates), no board frame to measure against; words from the engine gate and the tray |
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
| DW-0201 | Step 1: add a passkey | built; **deviation (D-P40-5, owner ruling: a passkey change proves the password)**: the step draws three things the board does not: the password field (Settings › Passkeys' field and words), "Add a passkey" busy while Windows' dialog is open (no words: Settings › Passkeys has none for it), and one refusal line in Settings › Passkeys' words (wrong password, cancelled dialog, offline, and the rest); the field moves the buttons down. `expected-red` on DW-0201 until the board draws all three: board, please draw them |
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
a dialog, the New file dialog (open, or asked for and opened by the next poll), Uploads, a drop hovering, the
protected-folder message, search, an opened photo) hides the tip and gets Esc first: for Files that list is the Files
window's own (`files::has_in_front`, the list its layers are built from). The keyboard is the card's while a tip shows
(focus on its button, Tab stays in the card, Enter presses), also when a tip comes back from behind what was in front
of it. No tip is drawn over a session the server ended. It ends when the account locks, signs out or changes, and
nothing is kept then.

**Deviation from the board (owner ruling, 2026-11-13).** The board says "Shown once per PC". The guide is done once
per ACCOUNT on each PC: an account that finished it (Done) or skipped it (Skip, Esc) does not get it again on this
PC, also after a sign-out and a sign-in; another account on the same PC gets it once. The mark is set when the guide
is finished or skipped, not when it starts (a guide a lock ended was not done). It is kept by the engine in the
account's own local state, keyed by the account's id, and read only for the signed-in account
(`setup/guide_marks.rs`, `engine_host/guide.rs`; the engine's `app/guide_mark.rs`). Only a definite "not done" starts
the guide; "not known" starts nothing. In a build on the real engine the first run is not bound yet, so today nothing
starts the guide by itself there: Settings › General › "Show the guide again" does. Tests: `setup/guide_tests.rs`,
`engine_host/guide_tests.rs`. Not built: the second way back the board names ("and Help"); the card's 22 pt button is
drawn here, not by the kit (a design-system need).

| Frame | What | State |
| --- | --- | --- |
| DW-0380 | Where the b is: "Beebeeb lives in the notification area", Got it | built in the main window (its lower right corner, clear of the details panel's buttons). Red on every dimension but text, icons and fills (225 of 225 geometry keys): the board composes a desktop with the window at 0.72 scale at (18, 18), the taskbar and the card by the tray; the app draws its window at full size. Its arrow points at nothing in the window until then: the card stands in the window's corner and the b is on the taskbar. Follow-up task 2479 (the card in a window of its own by the notification area) |
| DW-0301 | Tip 1: New | built; geometry, text, icons, fills, frame tiles 0; 3 pixel keys open: `coach-ring` (New's top edge at 40.75 pt: the board paints it from device row 81, Rune from 82, a 1 px step inside the ring), `coach-dot-0`, `coach-dots` |
| DW-0302 | Tip 2: a file's row (left out when the folder shows no file) | built; 1 pixel key open (`coach-dot-1`) |
| DW-0303 | Tip 3: Shared, on the Shared view | built; 2 pixel keys open: `coach-dot-2`, and `card-0-tile` (under the card: the card stands 0.54 low, measured; 0.5 of that is the sidebar's rows, see below, and the other 0.04 has no cause found yet; in dark also the tip's words, which Rune draws heavier light-on-dark than the board: 4,208 against 3,731 bright pixels in the crop's text rows) |
| DW-0304 | Tip 4: Photos, on the Photos view | built; 1 pixel key open (`coach-dot-3`) |
| DW-0305 | Tip 5: the gear | built; 2 pixel keys open (`coach-dot-4`, `coach-dots`) |

Measured on the Linux build VM, light and dark the same keys (2026-11-13). The open `coach-dot-*` keys (and
`coach-dots`, whose end is the lit dot) are the 6 pt dot's anti-aliased edge: the board's edge pixels are lighter
(86 / 121 / 186 against Rune's 74 / 107 / 169 at 2x), as `status-dot` in DW-A07; Rune's drawing, not this group's. The
sidebar's rows stand 0.5 pt low in every Files-window frame (`drive-label` is 12 high, the board's 11.5: `ui/sidebar.rs`),
and the ring and card of tips 3 and 4 follow them. Measured: with `drive-label` at 11.5 (an experiment in the VM's
checkout, not committed) DW-0303 light has 1 pixel key open (`card-0-tile` passes), dark still 2 (the words above).
`coach-ring` in DW-0301 is New's fill inside the ring (the Files window's New is `ui/toolbar.rs`, key `new-btn`; `viewers/chrome.rs` has the open-file window's own copy, which is not on screen here; Rune rounds its 40.75 pt top
edge to the next device row); the ring itself matches the board row for row. The words of DW-0380 are the board drawing's (`guide::TRAY_TEXT`); the
frame's note in `frames.json` has other words (decision 2478).

States with a rule and no drawing, built from the rule's words (tests only): the compare view's Stacked and Hold to
flip; "You're set up" after Not now or Skip setup ("Not added", "Off"); Turn on backup with no folder on (disabled).

States the board gives a rule and no drawing, built from the rule's own words (no golden, tests only):
DW-A07 with the pill reading "Reconnecting…"; DW-A08's error state "Approved, but the keys did not arrive.
Nothing was signed in. Ask again."; DW-A21's wait counting down in the message; a code being checked (the digits
dimmed, "Verifying…"); a wrong backup code; a code, a phrase or a vault password while offline or after too many
tries (the sign-in form's words for the same problem; "Too many tries. Wait an hour."; "Too many attempts. For
security, wait an hour and try again."); a vault password or a new password the engine refuses.

## Flagged deviations (states the board has no frame for; CP2 rows 3.4 and 3.5)

- **The new password's answers (DW-A49, row 3.5).** The board draws no error on DW-A49, and every refusal used to show
  the sign-in form's "We couldn't sign you in. Nothing was unlocked.", which was untrue when the password had changed.
  Each answer now has its own words (`view_recover.rs`): too short and breached under the field ("Nothing was
  changed"), and a banner above the fields, as DW-A48 draws its own, for Beebeeb unreachable (or a breach check that
  could not be made: the engine names both `Offline`), too many tries, an ended recovery and a step that did not
  finish. Past the registration request the window cannot tell whether the password changed,
  so those words are true either way ("If your new password was already set, sign in with it"). An ended recovery
  turns "Set new password" off and adds "Sign in". No countdown is promised for the wait (none runs).
- **Password updated, sign in to open the vault (`Screen::PasswordChanged`, row 3.5).** The password WAS changed and
  the account asks for a normal sign-in with it (two-factor, or an older server), so nothing was unlocked. DW-A50's
  layout (green check, "Password updated") with the words that are true here, and one button, "Sign in", to the form
  with the address kept. DW-A50 itself ("Your vault is unlocked") is drawn only when the recovery signed in. Board,
  please draw it.
- **A request lost after its code was shown (row 3.4).** The connection dropped (a new connection is a new code, so it
  cannot be joined again), or Beebeeb ended the request with an error. It was drawn as DW-A10's "This code expired".
  Now: DW-A10's layout with "This request was lost" / "Nothing was signed in. Ask for a new code." and the pill "Lost"
  ("lost" is DW-A07's own word for a request that ends). DW-A10 itself is unchanged.
- **An approval this PC did not take (`Screen::NotSignedIn`, row 3.4).** It was drawn as DW-A08's error state,
  "Approved, but the keys did not arrive", which is untrue: the keys arrived. Laid out as that error state, with the
  words of what happened: "Approved, but this PC holds another account" / "Nothing was signed in on this PC, and no
  keys were kept." when its stored session or its local files are another account's; "Approved, but this PC did not
  sign in" / "Nothing was signed in. Ask again." when it moved on while it waited (a sign-in, a lock or a sign-out
  here) or could not store the session. No account is named.
- **DW-A08 is seen after a real approval (row A08).** The engine opens the session before the request's answer reaches
  the window. `Ui::holds_open_gate` is true on the waiting request and on "Approved"; the poll hands it to
  `AppMachine::hold_sign_in`, and `AppMachine::screen` keeps the sign-in window for an open (Ready) gate while it holds.
  `follow_gate` does not reset those two screens over Ready. "Open Drive" clears the screen, so the gate decides alone.
  Other gates are never held.

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

## Screen lock: password (app design, no board frame)

Core's contract (`2026-11-13-two-level-lock-design.md`, section 7) has no frame for the screen lock's password
fallback: it is DW-A30's screen with the lock's words, in front of the lock screen (as #414 built it). The sentences
under its field are this app's own words, proposed here; each is drawn for its outcome by a test in `lock_tests.rs`
(`screen_password_*`). `{w}` is "30 seconds", "1 minute", "5 minutes", "15 minutes" or "1 hour", rounded up from the
engine's wait. While a wait runs, Unlock is disabled and the sentence counts down once a second.

| Outcome (`Engine::unlock_screen_with_password`) | What the lock screen says |
|---|---|
| `Opened` | nothing: the window opens |
| `Wrong`, no wait | "That password isn't right." |
| `Wrong`, a wait | "That password isn't right. Try again in {w}." |
| `Wait` | "Too many wrong tries. Try again in {w}." |
| `ServerLimit` | "Beebeeb is checking too many passwords right now. Try again in {w}." / "... Try again in a moment." |
| `LocalCheckOff` | "That password isn't right. This PC no longer checks your password offline: connect to the internet or use Windows Hello." |
| `OfflineNoRecord` | "Beebeeb can't be reached, and this PC can't check your password offline yet. Connect to the internet or use Windows Hello." |
| `AccountUpgradeNeeded` | the vault level's screen, starting with "Sign in again to finish updating your account." |
| `SessionEnded` | the vault level's screen, starting with "Your session ended. Enter your password to unlock and resume sync." |
| the press could not be checked | "Beebeeb couldn't check that password. Nothing was unlocked." |

The typed password is wiped when it is sent, on Back, and when the lock level changes.

Decision (lead, 2026-11-15 02:05, PR #428): **DW-0009 is out of the 0-failing set, on purpose.** The board draws
Windows' own "Windows Security / Making sure it's you" prompt over the locked window, with sample sidebar counts and
an account row. Windows draws that prompt, not the app, and the app's locked window shows no sample data: a fixture
that faked either would be a lie. The frame stays red in `compare.sh` (geometry 19 failing, measured 2026-11-15). It is
verified on the native rung: a person looking at the real Hello prompt at the first open of the app lock.
