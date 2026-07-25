# Beebeeb Desktop 0.8.1 — consistent error messages everywhere

A small consistency fix: every error message in the app now appears the same way, instead of three different ad hoc styles depending on which screen you were on.

### What's New

- Nothing new — this release is entirely about making existing errors look and behave consistently.

### Bug Fixes / Hardening

- **Consistent error presentation.** Task 0.6.0 introduced a shared toast notification system but only migrated 4 of the files that still hand-rolled their own error UI. This release finishes that migration for sign-in, first-run setup, the known-folder backup onboarding flow, the Explorer/Windows integration and launch-at-login settings, and several actions in the main file view (opening the sync folder, toggling a backed-up folder, restoring a file from Trash). Errors that need to stay next to the thing you're fixing — like the recovery-phrase unlock screen, a folder still loading, or the Trash list itself failing to load — were deliberately left as they were; only genuine one-off action failures moved to a toast.
- **Fixed a real bug along the way:** restoring a file from Trash that failed used to silently replace the entire Trash list with a "Could not load Trash" error card, even though the list had loaded fine — only the restore itself had failed. The restore failure is now its own toast, so the list stays visible.

### Verification

- `bunx tsc --noEmit` — clean. `bun run lint` — exits 0 (the accessibility/hooks gate from 0.6.0). `bun test` — 22 passed, 0 failed.
- Every changed file was reviewed directly, not just the summary: confirmed each case kept inline is genuinely load-bearing (gates other UI, or is a load-failure state, or is a form the user retries in place), and confirmed no leftover unused code from the removed error states.
- Not verified: this is a presentation-only change with no functional behavior change, so no real-hardware smoke test was run specifically for it — it ships alongside the existing outstanding real-hardware verification gate for this release cycle.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
