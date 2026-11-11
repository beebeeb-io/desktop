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
| DW-A63 | The account was deleted | not built; golden captured; `Intent::AccountDeleted` leads to it |
| DW-A23, DW-A24, DW-A25 | The 12 words on a new device, wrong words, no way in | not built; goldens captured |
| DW-A26, DW-A27, DW-A28 | Two-factor code, wrong code, backup code | not built; goldens captured |
| DW-A29, DW-A30 | Passkey, passkey then password | not built; goldens captured |
| DW-A45, DW-A46 | Forgot password: options, the honest dead end | not built; goldens captured |
| DW-A47 to DW-A50 | Recovery with the phrase, new password, done | not built; no goldens yet |
| DW-0009, DW-0014 | Locked, no plan yet | not built; no goldens yet |
| DW-0380, DW-0301 to DW-0305 | The first-time guide | not built; no goldens yet |

Built: 15 of 59. Not built: 44 (12 with goldens captured, 32 without). Lane f-signin-rest (below) adds the
first run on Windows: 11 more frames built, 26 of 70 in all.

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

Still not built after this lane (33 of the group's frames above): DW-A11 to A17, A23 to A30, A31 to A44, A45 to A50,
A63, DW-0009, DW-0014, DW-0380, DW-0301 to 0305.

States the board gives a rule and no drawing, built from the rule's own words (no golden, tests only):
DW-A07 with the pill reading "Reconnecting…"; DW-A08's error state "Approved, but the keys did not arrive.
Nothing was signed in. Ask again."; DW-A21's wait counting down in the message.

## For whoever builds the rest

- Keys for every `.au-mac` frame are already in `scripts/keymap-signin.js` (digits, words, cards, lists, strength bars).
- `ui::au` has the window, heading, buttons, fields, banners, pills, cards, lists, hero tile and ring. Missing for the
  rest: the six digit cells (`.au-digit`), the word grid (`.au-word`), the strength bars, a stand-alone paragraph.
- `SignInSource` already carries the calls and events of the frames above (`begin_two_factor`, `begin_restore`,
  `begin_passkey`, `begin_unlock`, `begin_recovery_verify`, `begin_set_password`); `Ui::apply` ignores their events
  until the frames exist.
- Secrets: a recovery word, a 2FA digit and a new password go in secure `TextEdit`s made ONCE in `Ui::new`, as
  the password is; a test types into the product's field and never makes one of its own (`tests.rs`:
  `a_password_typed_into_the_products_field_is_in_no_tree_and_no_label` and
  `a_shown_password_is_not_noted_measured_or_copied` are the patterns to copy for each).
- An engine answer is applied only by the wait it answers (`Ui::apply`: every arm has a guard); a new wait gets a
  guard and a line in `answers_of_a_request_already_left_change_nothing`.
