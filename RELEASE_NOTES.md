# Beebeeb Desktop 0.8.3 — closed a local privilege-escalation gap in the sync daemon

This release fixes a security issue in the background sync daemon's internal communication channel (used by the Linux/macOS file-system integration to ask the daemon to decrypt and write a file to disk). It required an attacker to already be running code as the same user as the daemon — it was not remotely exploitable, and there is no evidence it was ever exploited — but a local attacker with that level of access could have made the daemon decrypt and write any of your files to a location of their choosing, including a race condition that could redirect where the plaintext landed even after the destination was checked. We're treating this as serious enough to ship on its own, ahead of the next batch of feature work.

### What's New

- Nothing new — this release is a single security fix.

### Bug Fixes / Hardening

- **The sync daemon's local communication socket now authenticates the calling process.** Previously, any process running as the same OS user could connect and issue commands with no credential check at all. The daemon now verifies the connecting process's identity before accepting any request, and the socket file's own permissions are explicitly hardened rather than left to the system default.
- **Closed a time-of-check-to-time-of-use (TOCTOU) gap in how the daemon writes decrypted files to disk.** The daemon validates a destination path before decrypting and writing to it, but the validation and the write are necessarily two separate steps. A local attacker could previously exploit that gap — by swapping a symlink into place at exactly the right moment — to redirect the decrypted output somewhere the daemon never intended to write. The fix eliminates the gap structurally rather than narrowing it: the daemon now resolves the destination directory once, holds it open, and writes through that held reference for the rest of the operation, so nothing can be swapped out from underneath it afterward. A related gap that could leave a decrypted file world-readable if an attacker had planted a file at the destination beforehand is also closed — the daemon now always resets the file's permissions to owner-only, regardless of whether the file already existed.
- This fix went through four rounds of independent adversarial security review before shipping, each one specifically trying to find a way around the previous attempt. The third review found a real gap in the second attempt, and on its own follow-up re-examination later retracted part of that finding as not actually exploitable — both the original finding and the correction are kept in this project's history rather than only keeping the parts that make a cleaner story.

### Verification

- `cargo test --locked` — 332 passed, 0 failed (up from the prior release's 321), including new tests that specifically exercise the fixed attack paths: a malicious request over the real IPC socket with a destination outside the allowed area, a symlink swapped into place mid-operation, a directory renamed mid-operation, and a file planted in advance with the wrong permissions. Each of these tests was independently proven load-bearing — the underlying defense was temporarily removed, the corresponding test was confirmed to genuinely fail, then the defense was restored and the suite confirmed green again.
- Four independent adversarial security reviews were run against this fix, each one starting fresh rather than confirming the previous round's assumptions. The first three each found a real, additional gap in the prior attempt; the fourth found none.
- Not verified: real-hardware device testing on Windows/Linux/macOS for this specific release. The fix affects only the Linux and macOS sync daemon (Windows uses a different, unaffected code path); macOS itself remains parked pending separate build tooling and does not ship from this release.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
