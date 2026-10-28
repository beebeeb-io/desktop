# Release blockers

Each item below must be resolved before a desktop release is cut. While this file lists an item,
the release checks refuse: the release workflow (`.github/workflows/release.yml`) and the macOS
release script (`scripts/release-macos-local.sh`) both fail closed. Each blocker is one list item,
a line that starts with `- `. The change that resolves a blocker removes its item.

- The macOS sign-out flow for Finder saves still waiting to upload shipped in 0.8.12 with 1 of its 12 device checks run (D1 passed; D2–D11 and D10b not run, waived for 0.8.12 only). No later desktop release ships until those cases pass on a signed build (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` §13.4), or the maintainers waive again for that release.

- Shared folders 2237d is incomplete: Task 5 is stopped on the missing authenticated chunk-format reader premise (see `docs/2237d-evidence.md`), and Task 6 depends on it. Before a compatible desktop release/server enablement, complete those tasks and Task 13's signed Finder/Explorer device rung: guest/member add, rename, move and parent rename, trash/restore, overwrite/version history, revoke plus move-out/restart, local dirty held edits, and three quiet ticks with zero echo uploads/errors. Record old-client behavior and Windows-native evidence with lead glegion; Rust fixtures do not clear this blocker.
