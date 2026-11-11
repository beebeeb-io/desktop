# Sign-in, gate and first run: the frame checklist (lane f-signin)

The Windows frames of the board's groups First run, Sign in, Continue as, Another device, Approving a sign-in,
Email and password, Two-factor and passkeys, Forgot password, Before Files, and The first-time guide: 45 frames.
"Create an account" (DW-A31 on) is not a desktop frame: its two entry points emit `Intent::CreateAccount`.

Built = drawn from `Ui`, reachable by its controls, a scene in `main.rs`, gated in light and dark.
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
| DW-A63 | The account was deleted | not built; golden captured; `Intent::AccountDeleted` leads to it |
| DW-A23, DW-A24, DW-A25 | The 12 words on a new device, wrong words, no way in | not built; goldens captured |
| DW-A26, DW-A27, DW-A28 | Two-factor code, wrong code, backup code | not built; goldens captured |
| DW-A29, DW-A30 | Passkey, passkey then password | not built; goldens captured |
| DW-A45, DW-A46 | Forgot password: options, the honest dead end | not built; goldens captured |
| DW-A47 to DW-A50 | Recovery with the phrase, new password, done | not built; no goldens yet |
| DW-0009, DW-0014 | Locked, no plan yet | not built; no goldens yet |
| DW-0380, DW-0301 to DW-0305 | The first-time guide | not built; no goldens yet |

Built: 15 of 45. Not built: 30 (12 with goldens captured, 18 without).

## For whoever builds the rest

- Keys for every `.au-mac` frame are already in `scripts/keymap-signin.js` (digits, words, cards, lists, strength bars).
- `ui::au` has the window, heading, buttons, fields, banners, pills, cards, lists, hero tile and ring. Missing for the
  rest: the six digit cells (`.au-digit`), the word grid (`.au-word`), the strength bars, a stand-alone paragraph.
- `SignInSource` already carries the calls and events of the frames above (`begin_two_factor`, `begin_restore`,
  `begin_passkey`, `begin_unlock`, `begin_recovery_verify`, `begin_set_password`); `Ui::apply` ignores their events
  until the frames exist.
- Secrets: a recovery word, a 2FA digit and a new password go in secure `TextEdit`s, as the password does
  (`tests.rs`: `a_password_is_in_no_tree_and_no_label` is the pattern to copy for each).
