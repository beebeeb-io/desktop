# The account inside settings: the frame checklist (lane f-account)

Source: `design/desktop-v2/frames.json`, Windows tab, the five account groups (29 frames, all `[W]`: the board
draws each `DM-Axx` for Windows under the same number as `DW-Axx`). Every frame is the 960 x 640 window
(a 958 x 638 client area), in the account kit's settings view (`.au-set`).

Status after the review of #298 (2026-11-12, `origin/v2-native` 598ed681 merged): 29 of 29 built. 58 gate runs (29
frames, light and dark): 52 with every line at 0 failing, 6 with a red pixel line, each named in its row (8 keyed
elements in all; no geometry, text, icon or fill line is red). The built frames are the scenes of `frames::account::FRAMES`.

## Account in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A51` | Who is signed in, what became of the recovery phrase, which Beebeeb products this account uses. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`item-0-0-avatar edge 10 > 9`: the rim of the 32 pt avatar disc; measured, the board's disc is 0.2 device px smaller under its ring) |
| `DW-A52` | Sign-in and security: everything about getting in, who else is in, and getting out. | built (light and dark: 0 failing on all six lines) |
| `DW-A84` | Security checkup: the account's security factors in one list, with a score (7 of 7). | built (light and dark: 0 failing on all six lines) |
| `DW-A84a` | The same page with one thing left: no passkey yet (6 of 7). | built (light and dark: 0 failing on all six lines) |
| `DW-A55` | Devices and sessions: see and end every session of the account. | built (light and dark: 0 failing on all six lines) |
| `DW-A56` | Confirm ending one device's sessions. | built (light and dark: 0 failing on all six lines) |
| `DW-A57` | Confirm signing out of this app only. | built (light and dark: 0 failing on all six lines) |
| `DW-A57a` | Confirm signing out of all Beebeeb apps on this PC. | built (light and dark: 0 failing on all six lines) |

## Name, email and password (5)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A64` | Change the account's display name. | built (light and dark: 0 failing on all six lines) |
| `DW-A65` | Change the sign-in email: the new address and the password. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`field-0-caret edge 10 > 9`: the caret itself is equal pixel for pixel; the last letter beside it is drawn heavier, light text on dark) |
| `DW-A66` | Prove the new address: the 8-digit code. | built (light and dark: 0 failing on all six lines) |
| `DW-A67` | Change the password. | built (light and dark: 0 failing on all six lines) |
| `DW-A68` | The current password was wrong: nothing changed. | built (light and dark: 0 failing on all six lines) |

## Two-factor and passkeys in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A69` | Turn on two-factor with an authenticator app: QR code, key, 6-digit code. | built (light and dark: 0 failing on all six lines) |
| `DW-A70` | The 8 single-use backup codes, shown once. | built (light and dark: 0 failing on all six lines) |
| `DW-A71` | Two-factor status, the backup codes left, and the ways to change it. | built (light and dark: 0 failing on all six lines) |
| `DW-A72` | Confirm turning two-factor off with a current code. | built; 1 pixel line red in each theme (`digit-2 edge 28 > 9` light, `18 > 9` dark: the inner line of a focused box on a half device pixel) |
| `DW-A73` | The account's passkeys, and adding one. | built (light and dark: 0 failing on all six lines) |
| `DW-A74` | Passkeys with none added. | built (light and dark: 0 failing on all six lines) |
| `DW-A75` | The menu on one passkey: Rename, Remove. | built (light and dark: 0 failing on all six lines) |
| `DW-A76` | Confirm removing a passkey. | built (light and dark: 0 failing on all six lines) |

## Sign-in activity (3)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A77` | Sign-ins and security changes, with time, country and address; the security-logging switch. | built (light and dark: 0 failing on all six lines) |
| `DW-A78` | The confirm before security logging stops. | built; light: 1 pixel line red (`digit-2 edge 28 > 9`); dark: 3 (`digit-2 edge 18`, `list-2-body edge 11`, `item-2-0 edge 11`: the bold first line of the confirm, light text on dark, where the confirm lies over that list's edge) |
| `DW-A79` | Sign-in activity while security logging is off. | built (light and dark: 0 failing on all six lines) |

## Delete the account (5)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A58` | Step 1 of 4: what goes, the 72 hours to undo, the password. | built (light and dark: 0 failing on all six lines) |
| `DW-A80` | Step 2 of 4: the second factor. | built (light and dark: 0 failing on all six lines) |
| `DW-A81` | Step 3 of 4: confirm by an email link. | built (light and dark: 0 failing on all six lines) |
| `DW-A82` | Step 4 of 4: the deletion is scheduled, when, and how to undo it. | built (light and dark: 0 failing on all six lines) |
| `DW-A83` | The undo worked: nothing was deleted. | built (light and dark: 0 failing on all six lines) |

## Approving a sign-in (7, lane signin-approving)

The board's group "Approving a sign-in": the approving device's side of DW-A07. `approve.rs` (state, rules, views),
`approve_tests.rs`. The code is a secret the screen shows as typed: one secure, shown `TextEdit`; DW-A12, DW-A13 and
DW-A14 are in the capture-exclusion decision (`Pane::shows_secret`, `Dialog::shows_secret`).

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A11` | Windows' toast: "Allow Drive on Anna's iPhone?" | OS-drawn toast, verified by a real-window capture (not done yet: phase 2 sends it). No gated scene: Windows draws it. Its words are `approve::notification`, bound to the golden by a test; a click is `approve::notification_clicked` (opens DW-A13 with an empty code field) |
| `DW-A12` | Approve a sign-in: type the code the other device shows. | built (light and dark: 0 failing on all six lines) |
| `DW-A13` | The review: what the server measured, what the device reports, the code, Approve and Deny. | built; light: 0 failing on all six lines; dark: 1 pixel line red, 2 keys (`list-0-body edge 10 > 9`, `item-0-1 edge 10 > 9`: where the dialog lies over that list's edge, as DW-A78) |
| `DW-A14` | Windows Hello is asking; what approving shares, in amber. | built; geometry, text, icons and fills 0 failing in both themes; pixels differ (71 keys light, 30 dark): the board draws Windows' own prompt and a second scrim over the frame (as DW-A04, DW-A29) |
| `DW-A15` | Approved: the new session, and where to end it. | built (light and dark: 0 failing on all six lines) |
| `DW-A16` | Denied: nothing signed in, no keys shared; what to check. | built (light and dark: 0 failing on all six lines) |
| `DW-A17` | This request expired (and "Already handled"). | built (light and dark: 0 failing on all six lines) |

States the board gives a rule and no drawing, built from the rule's words (tests only): a code nothing waits for
("We could not find a waiting request with that code. Check it on the other device."); the red "Unusual place" note
above the facts; "Already handled" in place of DW-A17's heading; Windows Hello cancelled (back to the review, nothing
sent); offline.

Esc: on the review it is Deny while the code field holds the keyboard. With the keyboard on a button the shell takes
Esc first and closes settings, as on every account page: the flow ends with nothing approved and nothing sent.
