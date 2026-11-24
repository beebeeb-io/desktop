# The account inside settings: the frame checklist (lane f-account)

Source: `design/desktop-v2/frames.json`, Windows tab, the five account groups (29 frames, all `[W]`: the board
draws each `DM-Axx` for Windows under the same number as `DW-Axx`). Every frame is the 960 x 640 window
(a 958 x 638 client area), in the account kit's settings view (`.au-set`).

Status after the review of #298 (2026-11-12, `origin/v2-native` 598ed681 merged): 29 of 29 built. Since then the group
"Approving a sign-in" was added at the end of this file: 7 more frames, 6 built and gated, 1 drawn by Windows (35 scenes in all). 58 gate runs (29
frames, light and dark): 52 with every line at 0 failing, 6 with a red pixel line, each named in its row (8 keyed
elements in all; no geometry, text, icon or fill line is red). The built frames are the scenes of `frames::account::FRAMES`.

Since 2026-11-13 Settings > Security is ONE page with frame ids of its own (`DW-S01` to `DW-S03b`, eight, all gated at
0 failing): the seven board ids that drew the old pane are retired ("Sign-in and security, one page" below).

## Account in settings (8)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A51` | Who is signed in, what became of the master key, which Beebeeb products this account uses. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`item-0-0-avatar edge 10 > 9`: the rim of the 32 pt avatar disc; measured, the board's disc is 0.2 device px smaller under its ring) |
| `DW-A52` | Sign-in and security: everything about getting in, who else is in, and getting out. | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S01`. Not a gated frame any more; the id stays a name the app's links understand |
| `DW-A84` | Security checkup: the account's security factors in one list, with a score (7 of 7). | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S01` (the checkup is on the page). Not a gated frame any more |
| `DW-A84a` | The same page with one thing left: no passkey yet (6 of 7). | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S01a`. Not a gated frame any more |
| `DW-A55` | Devices and sessions: see and end every session of the account. | built (light and dark: 0 failing on all six lines) |
| `DW-A56` | Confirm ending one device's sessions. | built (light and dark: 0 failing on all six lines) |
| `DW-A57` | Confirm signing out of this app only. | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S02` (the same confirm, over the one page). Not a gated frame any more |
| `DW-A57a` | Confirm signing out of all Beebeeb apps on this PC. | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S02a`. Not a gated frame any more |

## Sign-in and security, one page (8; a design of this app's, no board frame)

`security_overview.rs`, tests in `tests_security.rs`. ONE scrolling page is what Settings > Security shows, in place
of DW-A52 (a list of rows) and the security checkup DW-A84 / DW-A84a (a second screen with the same subjects):

- the score card: the ring, "N of M security steps done", and what is left to do, each with the button that fixes it
  (the first is the page's one primary action);
- Sign-in: Password, Two-factor authentication, Passkeys, each with its state and the mark of a thing that is done;
- This PC and your devices: App lock with its switch and delay, Devices and sessions with how many are signed in,
  Approve a sign-in;
- Recent sign-in activity: the last three, then "All sign-in activity";
- Sign out (two confirms, which wait for the answer) and Delete, at the bottom.

Every row still opens its existing page. The page is drawn from `SecurityFacts`, where each part is known or not:
"Not known" for a part nobody read, "Not available yet" (and no link, switch or button) for what a build has not
bound; a row with a read state says that state alone; no score, no ring fill and no mark from a value that was not
read. On the real engine today the page knows two-factor (the profile's own mark; absent: not known) and how many
are signed in (one per session: "N signed in", never "devices"; "Loading" while that list is out; never the length
of a list read before a sign-out); the rest says so.

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-S01` | The page with everything done (7 of 7). | built (light and dark: 0 failing on all six lines) |
| `DW-S01a` | One thing left: no passkey yet (6 of 7), with its Add button in the score card. | built (light and dark: 0 failing on all six lines) |
| `DW-S01u` | The page of a source that knows only two-factor and how many are signed in (the running app today). | built (light and dark: 0 failing on all six lines) |
| `DW-S02` | Confirm signing out of this app only (was DW-A57). | built (light and dark: 0 failing on all six lines) |
| `DW-S02a` | Confirm signing out of all Beebeeb apps on this PC (was DW-A57a). | built (light and dark: 0 failing on all six lines) |
| `DW-S02w` | The confirm while its sign-out is out: it says so and takes no press. | built (light and dark: 0 failing on all six lines) |
| `DW-S03` | App lock's delay menu, open (was DW-0716). | built (light and dark: 0 failing on all six lines) |
| `DW-S03b` | App lock off: no time choice (was DW-0716b). | built (light and dark: 0 failing on all six lines) |

Deviation (lane, 2026-11-15): on the real engine App lock's switch is bound (core's `lock.app_lock`; off asks Windows
Hello first) but its delay is not: the away lock that "locks again after" sets has no idle timer in this version. So
the running app draws the row ON as "Lock Beebeeb · Windows Hello" with the switch and NO time choice (never a delay
nothing acts on), and Setup's step 2 draws no "Lock again after" row. The board's frames (`DW-S03`, `DW-S03b`,
`DW-0202`) keep theirs and still gate at 0. Ends when the away lock lands.

Ended (lane w5, 2026-11-15, decision 2519): the away lock landed. The delay is the engine's entry `lock.after` (five
minutes while nothing is stored), read with the switch and drawn as the board draws it, only while app lock is on;
Setup's step 2 draws its row (with the switch on) and writes it. A longer delay asks Windows Hello first, as off does;
a stored value the menu does not have is drawn as "Not known". "Immediately" locks when the main window closes to the
tray and after one minute without input while it stays (no time at all cannot be told from reading). The shell
measures (Windows' whole-PC idle, its session lock); the engine compares and locks through core's `lock_after_away`.

### App lock with the account password: not on the board (D-2)

The board's rule (DW-0716, Rules) is "The Windows app's password works when Windows Hello doesn't"; its Actions say
turning app lock off or changing the delay asks Windows Hello first. No board frame draws the password step: decision
D-2 of wave 2 (lead, 2026-11-15) has it drawn from the kit until the board gets the frame in the next designs PR.
`app_lock_password.rs`, bound in `engine_host/app_lock.rs`, tests in `engine_host/app_lock_password_tests.rs`.

- When Windows Hello cancels, fails or is not set up on this PC for turning app lock off or a longer delay, the Lock
  Beebeeb row offers "Use your password" (a link, the lock screen's words) while Sign-in and security shows.
- The step is the account kit's password confirm (DW-A78's): "Confirm it's you", one line for the change ("Turning off
  app lock needs your account password." / "A longer lock delay needs your account password."), "Account password",
  Continue and Cancel. While the password is checked, Continue says "Checking…" and nothing closes the step.
- The sentence under the field after an answer that keeps the step open is the lock screen's for the same outcome
  ("Screen lock: password" in `frames/signin/FRAMES.md`), without a prefix; an answer that ends the step is a toast
  that starts "App lock is still on: " or "The lock delay was not changed: ".

| Scene (no board id, no golden, not gated) | What it is |
| --- | --- |
| `APPLOCK-password-offered` | The page after Windows Hello did not confirm: "Use your password" in the Lock Beebeeb row. |
| `APPLOCK-password` | The step for turning app lock off, a password typed. |
| `APPLOCK-password-wrong` | After a wrong password: the field empty, "That password isn't right." under it. |
| `APPLOCK-password-checking` | The password is being checked: Continue says "Checking…", the field is frozen. |

Decision (lane, 2026-11-13): DW-A52, DW-A84, DW-A84a, DW-A57, DW-A57a, DW-0716 and DW-0716b are retired by the
owner's ruling of 2026-11-13 (option C of three rendered options: one page), because the two screens listed the
same subjects twice and the pane was a menu that said nothing about the account's state. The ruling wins over the
old board frames. Their ids stay names the app's links understand (`go`); they are no scenes and are not gated.
Their board goldens are left in `golden/` unread (not deleted: a deletion is the owner's call).

Decision (lane, 2026-11-14): the goldens of `DW-S..` are the APP's own renders of the approved page
(`scripts/golden-from-native.py`: the app's element export in the board golden's shape; the picture is `--snap`),
because the board has no drawing of this page and the lead ruled that the new ids are gated. What such a golden
is: a record of the approved render that fails when the page changes afterwards. It does not say the app matches
a design. Proven able to fail: with the score card's padding changed, `DW-S01` reads `geometry: 155 checked, 92
failing`. When the board draws the page, capture these goldens from the board instead.

Decision (lead, 2026-11-15 02:05, PR #428): the goldens of `DW-S01u` (both themes) are recaptured from the app with the
honest wording on the "Approve a sign-in" row, "Not available in this version yet. Approve on the web." (the triage's
honest state, `TRIAGE-fixture-only-1114.md` section 4), in place of the old sub "Let another device sign in" with the
value "Not available yet". An intended change, not a regression: the page has no board frame, so its golden is the
app's own record of the approved render (above). `compare.sh DW-S01u` reads 0 failing in both themes after it.

Decision (lane, 2026-11-14): the steps that are DONE are not listed on the page (the checkup listed all seven):
the ruling's page shows the count, what is left, and the three sign-in rows. Recorded here as a removed
behaviour; a "done" list in the score card is a follow-up if the owner wants it.

## Name, email and password (5)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-A64` | Change the account's display name. | built (light and dark: 0 failing on all six lines) |
| `DW-A65` | Change the sign-in email: the new address and the password. | built; light: 0 failing on all six lines; dark: 1 pixel line red (`field-0-caret edge 10 > 9`: the caret itself is equal pixel for pixel; the last letter beside it is drawn heavier, light text on dark) |
| `DW-A66` | Prove the new address: the 8-digit code. | built (light and dark: 0 failing on all six lines) |
| `DW-A67` | Change the password. | built (light and dark: 0 failing on all six lines) |
| `DW-A68` | The current password was wrong: nothing changed. | built (light and dark: 0 failing on all six lines) |

### States of these pages with no board frame (lane w1, 2026-11-15; desktop#427's follow-up)

The board draws none of these; each sentence is this app's, and says only what is true:
- Change email while its code is being sent: the fields frozen, "Leave this page", and under the address
  "A code is still being sent. Wait for it to finish." DW-A66 names the address Beebeeb sent the code to (its own
  word), or "your new address" when it is not known; never what the field holds.
- A password change answered after the person left its page: a notice over the window, whatever it shows
  ("Your password was changed" / "Your password change could not be confirmed" / "Your password was not changed",
  with Show), and the full sentence on the Change password page, once.
- Beebeeb refusing a new password (the same as now, too weak, found in a breach): "Beebeeb did not accept the new
  password. Nothing was changed. Choose a different one." under New password.
- A code confirm or the first code while the same call is still out: "Your earlier request is still being made.
  Wait for it to finish."; the email's code: "Your code is still being checked. Wait for it to finish."
- With no open session every one of these calls says the page's Offline line and asks nothing.

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
| `DW-A58` | Step 1 of 4: what goes, the time to undo, the password. | built (light and dark: 0 failing on all six lines) |
| `DW-A80` | Step 2 of 4: the second factor. | built (light and dark: 0 failing on all six lines) |
| `DW-A81` | Step 3 of 4: confirm by an email link. | built (light and dark: 0 failing on all six lines) |
| `DW-A82` | Step 4 of 4: the deletion is scheduled, when, and how to undo it. | built (light and dark: 0 failing on all six lines) |
| `DW-A83` | The undo worked: nothing was deleted. | built (light and dark: 0 failing on all six lines) |

Flagged deviations (lane p40-delete-account, 2026-11-15, lead rulings 16:35 and 17:30):
- D-P40-11: core and the server require the step-up for a signed-in cancel; board update after 0.9.0. Cancel deletion
  (DW-A81) and Undo deletion (DW-A82) open a password dialog over the page (`Dialog::DeletionPassword`), which the
  board does not draw.
- Board says 72 hours; the server keeps the account for 30 days; board update after 0.9.0. The copy names no length of
  time: DW-A58 and DW-A25's row say it can be undone "until a date we show you", the Security page's Delete row "for a time", and
  DW-A82 names Beebeeb's own `cancel_until` ("Your account will be deleted after {date}."; the note under it drops "72
  hours" and "within 30 days").
- DW-A82's banner: the board says "We sent a second email … with a link to undo it"; Beebeeb sends no email at the
  confirm (the first link undoes it too), so it says "The link in the email we sent to {email} undoes it too."
- DW-A13 draws a Password field under the code (D-P40-10); DW-A14 does not.

## Drive's own on these panes (3, lane settings-rest, 2026-11-12)

The board files these three under "Settings, inside the window"; they are states of this group's panes, so they are
built here (`plan.rs`, `app_lock.rs`) and are scenes of `frames::account::FRAMES`.

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-0708` | The plan, read only: a dialog over Account, from the plan card. | built; gate red, light and dark: the board draws an older Account pane behind the dialog (the plan card on top, no identity rows, a 540 pt column in Drive's settings kit), the app has one Account pane (DW-A51). In the gate, 9 of the dialog's 16 keys are at 0 (`scrim`, `dlg`, `dlg-title`, `dlg-text-0`, `dlg-foot`, `btn-0`, `btn-1` and the two labels) and 7 are RED: its list (`list-1-body`, `item-1-0`, `item-1-1` and their titles and values), which the board numbers `list-1` and the app `list-3` (the real pane has lists 0 to 2). Mapped by hand, `list-1` to `list-3`, those 7 are within 1 px; the gate does not measure that |
| `DW-0716` | App lock's delay: the pop-up open on Immediately, 5 min, 30 min, 60 min. | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S03`. Before that: built; gate red, light and dark: the board draws this frame in Drive's settings kit (28 pt sidebar rows, a 540 pt column) and DW-A52, "the same pane", in the account kit (30 pt, 520 pt). The app has ONE Security pane, DW-A52's, which gates at 0. DW-A52 itself still gates at 0 with the menu's anchor in its row. In the gate the menu's 10 keys are RED on geometry: the menu hangs 4 pt inside the card's trailing edge, the card is 520 pt here and 540 pt on this drawing, so the menu is 20 px to the left of the board's. Measured by hand as an offset from the pop-up button they are within 1 px; the gate does not measure that |
| `DW-0716b` | App lock off: no time choice. | retired by the owner's ruling 2026-11-13 (option C), replaced by `DW-S03b` |

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
