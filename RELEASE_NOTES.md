# Beebeeb Desktop 0.8.2 — fixed a rare cause of stuck uploads

A subtle bug meant a small fraction of failed uploads could get stuck instead of automatically retrying. This release fixes it.

### What's New

- Nothing new — this release is a single reliability fix.

### Bug Fixes / Hardening

- **Fixed a rare misclassification that could permanently stall a failed upload.** When an upload step failed, desktop decides whether to retry automatically or pause and ask for your attention, based on reading the error message. A bug in that logic meant it was accidentally also reading part of the network address the request had been sent to. On the rare occasion that address happened to contain digits like "401" or "403" — the same digits used for "unauthorized" and "forbidden" errors — a perfectly ordinary, retryable failure (like a brief server hiccup) could be misread as an authorization problem and paused instead of automatically retried. This was rare enough that it surfaced only once in this project's entire test history, and only in our automated testing infrastructure — but the same bug could in principle affect a real upload, so it's fixed now: the retry logic no longer looks at any part of the network address, only the actual error itself.
- This is also what was behind an intermittently failing automated test that came up during this release cycle — the test failure and the reliability issue were the same root cause, now both fixed together.

### Verification

- `cargo test --locked` — 321 passed, 0 failed, including a new test that specifically pins several previously-mismatched cases to the correct behavior.
- The specific previously-flaky test was run 45 additional times (30 by the engineer, 15 independently by the lead) with zero failures, and the exact failure was independently reproduced and confirmed fixed by deliberately forcing the unlucky condition that caused it.
- This fix ships specifically so it goes through a real GitHub Actions test-gate run, since the original bug only ever manifested in CI and never locally across 43+ attempts — local passing alone wasn't considered sufficient confidence.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
