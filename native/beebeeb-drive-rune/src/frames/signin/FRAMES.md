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
| DW-A11 to DW-A17 | Approving a sign-in (7 frames) | not built; no goldens yet |
| DW-A18 | Sign in: unlocking the vault | built, 0 |
| DW-A19 | Sign in: email and password do not match | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A20 | Sign in: offline | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A21 | Sign in: too many tries from this network | built; light 0, dark 1 pixel line open (`card-0-avatar`) |
| DW-A22 | Sign in: the lock explained | built; the note wraps one word later than the board (3 pixel lines), dark also `card-0-avatar` |
| DW-A31 to DW-A44 | Create an account: email, verify, terms, password, phrase, this device (14 frames) | not built; no goldens yet; "Create a Beebeeb account" and "Create an account" emit `Intent::CreateAccount` for DW-A31. Which lane builds them is the lead's call |
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
| DW-0380, DW-0301 to DW-0305 | The first-time guide | not built; no goldens yet |

Built: 30 of 59 here, and the 11 frames of the first run below: 41 of 70. Not built: 29, none with a golden yet
(DW-A11 to A17, DW-A31 to A44, DW-0009, DW-0014, DW-0380, DW-0301 to 0305).

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

`ui/au_entry.rs` has the digit boxes, the word boxes, a field drawn holding the keyboard, and links `au::link` does
not draw. Scenes of DW-A23 and later are `signin::FRAMES` (`frames/mod.rs`), not lines in `shell/mod.rs`.

## For whoever builds the rest

- Keys for every `.au-mac` frame are already in `scripts/keymap-signin.js` (digits, words, cards, lists, strength bars).
- Still missing for sign-up (DW-A31 to A44): the terms checkboxes, the phrase shown for writing down (the word
  boxes read-only, a copy and a print action), the device-name step. The strength meter is `view_recover.rs::strength`.
- A secret that the screen shows as typed (a code, a recovery word, a backup code) is a secure `TextEdit` made ONCE
  in `Ui::new`, revealed, or a secure text node (`au_entry::shown_secret`); it is never an element label, never
  measured with `rune::text::advance`, never copied into a `String`. `steps_tests.rs`:
  `a_typed_code_and_typed_words_are_in_no_tree_and_no_label` is the pattern to copy.
- A headless render has no keyboard focus: a frame the board draws with a focused field names it in
  `steps::Pictured` (scenes only; the first key or press clears it).
- An engine answer is applied only by the wait it answers (`Ui::apply`, `Ui::apply_step`: every arm has a guard); a
  new wait gets a guard and a line in `back_from_the_code_discards_the_partial_sign_in_and_a_late_answer_changes_nothing`.
