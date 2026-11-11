# The account inside settings: the frame checklist (lane f-account)

Source: `design/desktop-v2/frames.json`, Windows tab, the five account groups (29 frames, all `[W]`: the board
draws each `DM-Axx` for Windows under the same number as `DW-Axx`). Every frame is the 960 x 640 window
(a 958 x 638 client area), in the account kit's settings view (`.au-set`).

Status at the end of this lane's run (2026-11-11): 8 of 29 built, 7 of them with every gate line at 0 failing in both
themes, 1 with its one red line named; 21 not built (hand-over in the pull request). The built frames are the scenes of
`frames::account::FRAMES`.

## Account in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A51` | Who is signed in, what became of the recovery phrase, which Beebeeb products this account uses. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`item-0-0-avatar edge 10 > 9`: the anti-aliased rim of the 32 pt avatar disc) |
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
| `DW-A64` | Change the account's display name. | not built (this lane stopped at its budget) |
| `DW-A65` | Change the sign-in email: the new address and the password. | not built (this lane stopped at its budget) |
| `DW-A66` | Prove the new address: the 8-digit code. | not built (this lane stopped at its budget) |
| `DW-A67` | Change the password. | not built (this lane stopped at its budget) |
| `DW-A68` | The current password was wrong: nothing changed. | not built (this lane stopped at its budget) |

## Two-factor and passkeys in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A69` | Turn on two-factor with an authenticator app: QR code, key, 6-digit code. | not built (this lane stopped at its budget) |
| `DW-A70` | The 8 single-use backup codes, shown once. | not built (this lane stopped at its budget) |
| `DW-A71` | Two-factor status, the backup codes left, and the ways to change it. | not built (this lane stopped at its budget) |
| `DW-A72` | Confirm turning two-factor off with a current code. | not built (this lane stopped at its budget) |
| `DW-A73` | The account's passkeys, and adding one. | not built (this lane stopped at its budget) |
| `DW-A74` | Passkeys with none added. | not built (this lane stopped at its budget) |
| `DW-A75` | The menu on one passkey: Rename, Remove. | not built (this lane stopped at its budget) |
| `DW-A76` | Confirm removing a passkey. | not built (this lane stopped at its budget) |

## Sign-in activity (3)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A77` | Sign-ins and security changes, with time, country and address; the security-logging switch. | not built (this lane stopped at its budget) |
| `DW-A78` | The confirm before security logging stops. | not built (this lane stopped at its budget) |
| `DW-A79` | Sign-in activity while security logging is off. | not built (this lane stopped at its budget) |

## Delete the account (5)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A58` | Step 1 of 4: what goes, the 72 hours to undo, the password. | not built (this lane stopped at its budget) |
| `DW-A80` | Step 2 of 4: the second factor. | not built (this lane stopped at its budget) |
| `DW-A81` | Step 3 of 4: confirm by an email link. | not built (this lane stopped at its budget) |
| `DW-A82` | Step 4 of 4: the deletion is scheduled, when, and how to undo it. | not built (this lane stopped at its budget) |
| `DW-A83` | The undo worked: nothing was deleted. | not built (this lane stopped at its budget) |
