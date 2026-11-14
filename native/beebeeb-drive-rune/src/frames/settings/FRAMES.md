# Settings, inside the window: the frame checklist (lanes f-settings and f-settings-rest)

Source: `design/desktop-v2/frames.json`, Windows tab, the three settings groups (50 frames). Account-related groups are a later wave.
Status after lane f-settings-rest (2026-11-12): 28 of 50 built, 23 of them with every gate line at 0 failing in both themes, 5 with the red lines named; 22 not built (3 belong to lane f-account's panes, 19 are in the pull request's hand-over).
Status after the Bandwidth lane (2026-11-12): Bandwidth's 6 and the speed test's 4 are built (`bandwidth.rs`, `bandwidth_data.rs`): 9 of the 10 with every gate line at 0 failing in both themes, 1 (`DW-0785d`, dark) with its red line named below. The speed test's steps are reached from Run test: the board fixture moves on with the window's clock.

## Settings, inside the window (13)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-0701` | Where Ctrl+, lands: the suite's settings sidebar and the General section. | built (light and dark: 0 failing on all six lines) |
| `DW-0781` | A new version is available: General says so with Download update. | built (light and dark: 0 failing on all six lines) |
| `DW-0751` | Versions to quote in a problem report, and where the code is. | built (light and dark: 0 failing on all six lines) |
| `DW-0782` | Report a problem the way the desktop app does: a support bundle the person can read first, and an email they send themselves. | built (light and dark: 0 failing on all six lines) |
| `DW-0708` | The plan, read only. | built on the account pane (`frames/account/plan.rs`, over DW-A51); the board draws them in the Drive settings kit: board follow-up task 2480 (`frames/account/FRAMES.md` has the counts) |
| `DW-0712` | What uses the account's space, and what Beebeeb keeps on this PC. | built (light and dark: 0 failing on all six lines) |
| `DW-0713` | How much room previews may take on this PC. | built; light and dark: 1 pixel line red (`slider-knob edge`: the knob's rim and shadow) |
| `DW-0714` | How much room downloaded files may take on this PC. | built; light and dark: 1 pixel line red (`slider-knob edge`: the knob's rim and shadow) |
| `DW-0780` | Remove downloaded copies to get space back on Windows. | built (light and dark: 0 failing on all six lines) |
| `DW-0716` | Lock Beebeeb's screens with Windows Hello: the one Security pane, choosing how soon it locks again. | built on the account pane (`frames/account/app_lock.rs`, the delay menu over DW-A52); the board draws them in the Drive settings kit: board follow-up task 2480 (`frames/account/FRAMES.md` has the counts) |
| `DW-0716b` | App lock is off: no time choice. The same pane as DW-A52. | built on the account pane (DW-A52 with the lock off); the board draws them in the Drive settings kit: board follow-up task 2480 (`frames/account/FRAMES.md` has the counts) |
| `DW-0750` | Which alerts Beebeeb sends on this PC. | built (light and dark: 0 failing on all six lines) |
| `DW-0750b` | Notifications are off for Beebeeb in Windows: say so, and that nothing arrives. | built (light and dark: 0 failing on all six lines) |
| `DW-0750n` | The account's switches could not be read: the rows say so, and the pane says it over itself with Try again, in view at any window size. A state of this app's own (no board frame); its golden is the app's own render (`scripts/golden-from-native.py`). | built (light and dark: 0 failing on all six lines) |
| `DW-0750u` | The switches are drawn but could not be confirmed with Beebeeb: said over the pane with Try again, and above it the toast of a change that was refused meanwhile. A state of this app's own (no board frame); golden as `DW-0750n`. | built (light and dark: 0 failing on all six lines) |

## Settings: Drive's own sections (5)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-0746` | Beebeeb in File Explorer, sync, and the space Beebeeb uses on this PC, in one place. | built (light and dark: 0 failing on all six lines) |
| `DW-0746b` | Turning off the File Explorer location warns about anything that would be lost. | built (light and dark: 0 failing on all six lines) |
| `DW-0744` | How photos preview: the size and format of the stored previews, and what that costs. | built (light and dark: 0 failing on all six lines) |
| `DW-0744b` | A change of size or format is confirmed with its cost. | built (light and dark: 0 failing on all six lines) |
| `DW-0745` | See a preview against the original before choosing a size. | built; light and dark: 3 pixel lines red (`item-3-1`, `item-3-2`, `list-3-body` edge: the rows are under the compare view and their edges run through the words at its foot, which Rune sets on whole device pixels) |

## Settings: what the desktop app does today (32)

| Frame | What it is | Status |
| --- | --- | --- |
| `DW-0785` | See what Beebeeb is sending and receiving now and over the last 20 hours, each device's sync, the speed limits and a speed test, in one place. | built (light and dark: 0 failing on all six lines) |
| `DW-0785b` | Nothing is moving: say so instead of showing zeros. | built (the head of the pane; the app draws the whole pane; light and dark: 0 failing on all six lines) |
| `DW-0785c` | A limit is on: say that the speed is held back on purpose, and by how much. | built (light and dark: 0 failing on all six lines) |
| `DW-0785d` | Choose how fast Beebeeb may upload from this PC. | built (the pane from its devices down); light: 0 failing on all six lines; dark: 1 pixel line red (`item-1-2-switch edge 13 > 9`: the switch is under the open menu) |
| `DW-0785e` | Right after sign-in there is no history yet: say when it starts. | built (the head of the pane; light and dark: 0 failing on all six lines) |
| `DW-0785f` | Sync on metered connections is off and the PC is on one: say that sync waits, why, and how to change it. | built (the head and the Limits; light and dark: 0 failing on all six lines) |
| `DW-0791` | The account's recent alerts inside the app, for when a notification was missed or dismissed. | built (light and dark: 0 failing on all six lines; the pill dots are vector circles since lane f-settings-rest) |
| `DW-0791b` | Nothing to show. | built (light and dark: 0 failing on all six lines) |
| `DW-0786` | The test runs: show which step it is on, so a long wait reads as progress. | built (the pane from its devices down; light and dark: 0 failing on all six lines) |
| `DW-0786b` | The last step: downloads. | built (the pane from its devices down; light and dark: 0 failing on all six lines) |
| `DW-0786c` | How fast this PC reaches Beebeeb, in three numbers. | built (the pane from its devices down; light and dark: 0 failing on all six lines) |
| `DW-0786d` | The test couldn't reach Beebeeb: say which part failed and what to do. | built (the pane from its devices down; light and dark: 0 failing on all six lines) |
| `DW-0783` | Where the storage goes: by type, and the files that take the most. | built; light 7 and dark 4 pixel lines red (the round ends of the 9 pt meter and share bars) |
| `DW-0783b` | Loading: the page's shape in skeletons, never a spinner on a blank page. | built (light and dark: 0 failing on all six lines) |
| `DW-0783c` | Couldn't load: say so plainly, with Try again. | built (light and dark: 0 failing on all six lines) |
| `DW-0783d` | Nothing stored yet. | built (light and dark: 0 failing on all six lines) |
| `DW-0784` | Choose which folders stay on this PC, many at once, and see what each choice does before it happens. | built (light: 7 and dark: 3 pixel lines red, all on Sync and File Explorer's own rows UNDER the dialog (`btn-2`, `item-2-1`, `item-2-2` and their parts): the dialog's text runs through their edges; the other five lines 0 failing, frame tiles 0 failing) |
| `DW-0784b` | Before downloaded copies leave the PC, say what leaves, that nothing is deleted, and what downloads. | built (light and dark: 0 failing on all six lines) |
| `DW-0784c` | The folder list couldn't load: nothing is changed. | built (light: 0 failing on all six lines; dark: 1 pixel line red, `Notifications-row edge 10 > 9`: the sidebar row is under the dialog's edge) |
| `DW-0784d` | No folders yet. | built (light: 0 failing on all six lines; dark: 1 pixel line red, `Notifications-row edge 10 > 9`: the sidebar row is under the dialog's edge) |
| `DW-0787` | Light, dark, or follow Windows. | built; dark: 1 pixel line red (`item-0-0-pop edge 14 > 9`: the pop-up button is under the open menu, its edge runs through the menu's text) |
| `DW-0788` | Choose where new files are stored, inside the EU. | built (light and dark: 0 failing on all six lines) |
| `DW-0788b` | A country is chosen; decide what happens if it can't take a file. | built (light and dark: 0 failing on all six lines) |
| `DW-0788c` | The chosen country can't take files right now: say where new files go meanwhile. | built (light and dark: 0 failing on all six lines) |
| `DW-0788d` | The locations couldn't load. | built (light and dark: 0 failing on all six lines) |
| `DW-0789` | Choose which releases this PC is offered. | built (light and dark: 0 failing on all six lines) |
| `DW-0789b` | What a version changes, before downloading it. | built; dark: 1 pixel line red (`item-1-1 edge 10 > 9`: the row is under the dialog, its edge runs through the dialog's text) |
| `DW-0790` | See what Beebeeb is doing on this PC, and save a support bundle you can read before you send it. | built (light and dark: 0 failing on all six lines) |
| `DW-0790b` | The bundle is saved: say where, and what happens next. | built (light and dark: 0 failing on all six lines) |
| `DW-0746c` | When File Explorer stops showing Beebeeb's files or their state, put Beebeeb back in File Explorer without losing anything. | built (light and dark: 0 failing on all six lines) |
| `DW-0746d` | The repair didn't finish: say that nothing needs undoing. | built (light and dark: 0 failing on all six lines) |
| `DW-0746e` | Sync is waiting because the PC is on a metered connection: say so where sync's status lives, and offer to sync anyway. | built (light and dark: 0 failing on all six lines) |
