# Beebeeb Desktop 0.4.0 — a sync ghost-row bug, found before it was reported

This release ships one real fix, found by deliberately auditing the sync engine's edge cases rather than waiting for a bug report. It also closes out a promise from 0.3.0: this release notes doc reports honestly that the roadmap's original plan for this cycle turned out to target code desktop doesn't even run — see "Not shipped" below.

### Bug Fixes / Hardening

- **Fixed a rare sync ghost-row bug: trashing a folder while a sync was mid-flight could leave its contents behind.** If you trashed a folder on one device at the exact moment another device's sync was mid-snapshot, the sync engine could see the folder's children in that snapshot without seeing the folder itself (a legitimate partial-sync race). The old logic re-rooted those children to the top of your file list instead of recognizing they belonged to a folder that no longer existed — so they'd survive as orphaned "ghost" entries instead of being cleaned up with the rest of the trashed folder. Fixed by keeping track of each file's last-known real location instead of guessing it belongs at the root when its parent goes missing mid-sync.
- Three other edge cases in the same sync logic were audited and confirmed already correct — a same-file trash/restore race, two rapid trash-then-restore cycles in a row, and an offline local edit colliding with a remote delete. Each is now backed by a dedicated regression test, not just a read-through.

### Verification

- `cargo test --locked` — 296 passed, 0 failed (up from 291 in 0.3.0; the 5 new tests cover the bug above plus the three confirmed-correct scenarios and one bonus conflict-safety check).
- The regression test for the fixed bug was proven genuine, not just present: temporarily reverting the fix made the test fail with the exact predicted symptom; restoring the fix made it pass again.
- Not verified: this bug requires a very specific timing race (a folder trash landing between two sync ticks on another device) that hasn't been reproduced on real hardware — the fix and its test are the verification here, since the race isn't practically reproducible by hand.

### Not shipped this cycle

The original plan for this release cycle was to fix known desktop sync-engine bugs referenced in older backlog items (0829, 0865). Before dispatching that work, we read the actual code those items describe and found it targets `beebeeb-sync`'s standalone sync-operation engine — a code path desktop doesn't drive; desktop's real sync logic lives in `engine_bridge.rs` and already has mature, tested remote-delete handling that those backlog items predate. Fixing the wrong code path would have been wasted work with no effect on this app, so we corrected course and audited the code desktop actually runs instead — which is what produced the fix above.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
