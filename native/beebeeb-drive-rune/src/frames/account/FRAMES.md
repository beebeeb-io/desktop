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

## Drive's own on these panes (3, lane settings-rest, 2026-11-12)

The board files these three under "Settings, inside the window"; they are states of this group's panes, so they are
built here (`plan.rs`, `app_lock.rs`) and are scenes of `frames::account::FRAMES`.

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-0708` | The plan, read only: a dialog over Account, from the plan card. | built; gate red, light and dark: the board draws an older Account pane behind the dialog (the plan card on top, no identity rows, a 540 pt column in Drive's settings kit), the app has one Account pane (DW-A51). In the gate, 9 of the dialog's 16 keys are at 0 (`scrim`, `dlg`, `dlg-title`, `dlg-text-0`, `dlg-foot`, `btn-0`, `btn-1` and the two labels) and 7 are RED: its list (`list-1-body`, `item-1-0`, `item-1-1` and their titles and values), which the board numbers `list-1` and the app `list-3` (the real pane has lists 0 to 2). Mapped by hand, `list-1` to `list-3`, those 7 are within 1 px; the gate does not measure that |
| `DW-0716` | App lock's delay: the pop-up open on Immediately, 5 min, 30 min, 60 min. | built; gate red, light and dark: the board draws this frame in Drive's settings kit (28 pt sidebar rows, a 540 pt column) and DW-A52, "the same pane", in the account kit (30 pt, 520 pt). The app has ONE Security pane, DW-A52's, which gates at 0. DW-A52 itself still gates at 0 with the menu's anchor in its row. In the gate the menu's 10 keys are RED on geometry: the menu hangs 4 pt inside the card's trailing edge, the card is 520 pt here and 540 pt on this drawing, so the menu is 20 px to the left of the board's. Measured by hand as an offset from the pop-up button they are within 1 px; the gate does not measure that |
| `DW-0716b` | App lock off: no time choice. | built: DW-A52 with the switch off; gate red for the same reason as DW-0716 |

Decision (lane, 2026-11-12): one Security pane and one Account pane, in the account kit, because DW-0716's own rule
says "Security is one pane (final review C3): DW-A52" and DW-0708's says it is "reached from the plan card Drive adds
to the shared Account page (DW-A51)". The three goldens were captured from drawings in the other kit; no tolerance
was loosened and no key dropped. To turn them green the BOARD has to draw these three in the account kit, and the
goldens be captured again: a board change, not this lane's (board follow-up task 2480).
Gate, light / dark, geometry failing: DW-0716 96 / 96, DW-0716b 83 / 83, DW-0708 59 / 59.
Of those: DW-0716 86 pane keys + 10 menu keys; DW-0716b 83 pane keys; DW-0708 52 keys of the pane behind + 7 keys of the dialog's list.
What task 2480 is expected to remove: all of them (a redraw in the account kit moves the card to 520 pt, so the menu follows, and gives the pane three lists, so the dialog's list becomes `list-3`). Expected, not measured: after the recapture, run the gate and correct these rows.
