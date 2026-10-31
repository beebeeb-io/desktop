# macOS redesign release rungs — 26 October 2026

Task 1683, lane l5-macos. These are **pending**, not evidence of a device run.
The full-speed program §5.6 delays device rungs to the release window. This run
was explicitly limited to type/lint checks and unit tests: no app bundle, launch,
signing, simulator, or E2E work was performed.

| Area | Required release evidence | State |
| --- | --- | --- |
| Slice 3 popover | Ten open/close clicks, both mouse buttons, two displays with different scale factors, notch and off-screen bounds; blur and Escape hide; gear menu remains usable; Cmd-comma opens Settings; Cmd-Q quits | Pending |
| Session and keys | Signed out and expired sessions open sign-in rather than a popover; Lock exposes one unlock action; missing keys continue in Set up this Mac in the existing window, including while a sheet is open | Pending |
| Review | A real Mac/web conflict resolved with each of the three choices; existing conflict copies survive retries and failures; failed choices remain actionable | Pending engine-owner safety integration and device run |
| Review navigation | Cmd-2, popover Review, and native notification activation reach Review in the one Beebeeb window; no conflict webview is created | Notification activation bridge still missing; other routes unit/source checked only |
| Trash and history | Restore from Trash; cancel permanent delete without changes; confirm delete affects only the named file; version history and restoration stay over Review | Pending |
| Home | Counts agree with the mirrored files; unavailable folder counts stay absent; attention rows/actions match Review; recent activity matches the popover; kept sign-out and repair rows remain visible | Pending; pinned-folder producer is separate engine/DTO work |
| Existing sign-out gate | Task 1887 D2–D11, D10b, D2b and task 2175 on the final integrated build; menu confirmation and Cancel included | Existing RELEASE_BLOCKERS.md remains in force |
| Launch | Finder/Dock reopen, closed-window frame restore, unplugged display, no launch window at login, no window after updater relaunch | Pending; working login-item mechanism is task 1845 |
| Account states | Checking, no-plan and update-required behavior from the authoritative account view | Task 1747 dependency |
| Updates, task 2176 | Stable download opens the stable disk image; beta/alpha open the advertised release; refusal to install remains clear and preserves the old app | Pending device rung; 469 integrated TS tests passed, 0 failed |
| In-place installer | The updater spike's signed-device proofs P1–P4 before choosing/implementing its recommended XPC installer | Outside this run; spec remains a read-only recommendation for the lead's decision |

Before merge, the remote wrapper must complete the Rust compile and targeted
native tests. It did not execute here: SSH to the test host was denied by the
managed session. Rustfmt parsed the host sources; that is not a Rust typecheck.
No macOS release-readiness claim follows from the TypeScript results alone.
