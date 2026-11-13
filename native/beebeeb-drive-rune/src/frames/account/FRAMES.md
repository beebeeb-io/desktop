# The account inside settings: the frame checklist (lane f-account)

Source: `design/desktop-v2/frames.json`, Windows tab, the five account groups (29 frames, all `[W]`: the board
draws each `DM-Axx` for Windows under the same number as `DW-Axx`). Every frame is the 960 x 640 window
(a 958 x 638 client area), in the account kit's settings view (`.au-set`).

Status after the review of #298 (2026-11-12, `origin/v2-native` 598ed681 merged): 29 of 29 built. Since then the group
"Approving a sign-in" was added at the end of this file: 7 more frames, 6 built and gated, 1 drawn by Windows (35 scenes in all). 58 gate runs (29
frames, light and dark): 52 with every line at 0 failing, 6 with a red pixel line, each named in its row (8 keyed
elements in all; no geometry, text, icon or fill line is red). The built frames are the scenes of `frames::account::FRAMES`.

Since 2026-11-13 Settings > Security is ONE page: DW-A52, DW-A84 and DW-A84a are replaced by it, and the gates of
the seven frames that draw it are red by design ("Sign-in and security, one page" below has the decision and the counts).

## Account in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A51` | Who is signed in, what became of the recovery phrase, which Beebeeb products this account uses. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`item-0-0-avatar edge 10 > 9`: the rim of the 32 pt avatar disc; measured, the board's disc is 0.2 device px smaller under its ring) |
| `DW-A52` | Sign-in and security: everything about getting in, who else is in, and getting out. | REPLACED by the one page (see "Sign-in and security, one page" below); the id shows that page; gate red by design |
| `DW-A84` | Security checkup: the account's security factors in one list, with a score (7 of 7). | REPLACED: not a screen any more; the id shows the one page; gate red by design |
| `DW-A84a` | The same page with one thing left: no passkey yet (6 of 7). | REPLACED: the id shows the one page with one thing left; gate red by design |
| `DW-A55` | Devices and sessions: see and end every session of the account. | built (light and dark: 0 failing on all six lines) |
| `DW-A56` | Confirm ending one device's sessions. | built (light and dark: 0 failing on all six lines) |
| `DW-A57` | Confirm signing out of this app only. | built; the confirm is the board's, the page behind it is the one page: gate red on the page's keys |
| `DW-A57a` | Confirm signing out of all Beebeeb apps on this PC. | built; as DW-A57: gate red on the page's keys |

## Sign-in and security, one page (a design of this app's: no board frame)

`security_overview.rs`, tests in `tests_security.rs`. ONE scrolling page is what Settings > Security shows, in place
of DW-A52 (a list of rows) and the security checkup DW-A84 / DW-A84a (a second screen with the same subjects):

- the score card: the ring, "N of M security steps done", and what is left to do, each with the button that fixes it
  (the first is the page's one primary action);
- Sign-in: Password, Two-factor authentication, Passkeys, each with its state and the mark of a thing that is done;
- This PC and your devices: App lock with its switch and delay (DW-0716's menu), Devices and sessions with the
  count, Approve a sign-in;
- Recent sign-in activity: the last three, then "All sign-in activity";
- Sign out (the board's two confirms, DW-A57 and DW-A57a) and Delete, at the bottom.

Every row still opens its existing page. The page is drawn from `SecurityFacts`, where each part is known or not:
"Not known" for a part nobody read, "Not available yet" (and no link, switch or button) for what a build has not
bound; no score, no ring fill and no mark from a value that was not read. On the real engine today the page knows
two-factor (the profile's own mark; absent: not known) and how many are signed in (one per session: "N signed
in", never "devices"); the rest says so.

Decision (lane, 2026-11-13): DW-A52 and DW-A84 / DW-A84a are replaced by this one page (the product owner's ruling
of 2026-11-13 21:58, option C of three rendered options), because the two screens listed the same subjects twice
and the pane was a menu that said nothing about the account's state.

Decision (lane, 2026-11-13): the goldens of DW-A52, DW-A84, DW-A84a, DW-A57, DW-A57a, DW-0716 and DW-0716b are
left as the board drew them: not recaptured, not deleted, no tolerance loosened, no key dropped, because a golden
is the BOARD's drawing and the board has no drawing of this page (one captured from the app could not fail). Their
gates are red until the board draws the page and the goldens are captured again: a board follow-up.

Gate on this branch, light / dark, geometry failing of checked (VM, 2026-11-13): DW-A52 66 of 114 / 66 of 114;
DW-A84 49 of 98 / 49 of 98; DW-A84a 51 of 100 / 51 of 100; DW-A57 66 of 124 / 66 of 124; DW-A57a 66 of 124 /
66 of 124; DW-0716 113 of 124 / 113 of 124 (96 before, see below); DW-0716b 100 of 111 / 100 of 111 (83 before).

`SEC-PARTLY-KNOWN` is a scene with no board frame and no golden: the page as a source draws it that knows only
two-factor and the devices (what the running app shows today), so it can be rendered and looked at.

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
above the facts, worded as an estimate (the country is derived from a network address); "Already handled" in place
of DW-A17's heading; Windows Hello cancelled (back to the review, nothing sent); offline at the lookup, and before
anything was sent ("Beebeeb can't be reached. Nothing was approved.").

Ends with no board frame and no gated scene (the security review of the frames PR; tests only, `request.rs` holds
the types): "Result not known" (the connection went after Windows Hello with the claim sent: the screen keeps
checking and only "approved" for this request settles it), "Not denied" (the denial could not be sent: Try again,
Close) and "Already approved" (Deny came after this PC's own approval went through). None of them holds the code.

Rules the review added: a lock, a sign-out or another account ends the flow (and the account forms) from the
shell's poll; Approve never holds the keyboard when the review opens and is the default only once the code was
typed on the review itself; every answer and every notification click names its request; a request's strings are
bounded by `SignInRequest::bounded` (one line, a cap in characters, no control, direction or hidden character).

Esc: the dialogs are modal. On the review Esc is Deny (the board); on a result it is the result's first way out.

Not tested, no fixture path: a request that runs out while it is only being looked at (`sign_in_request_gone`).
