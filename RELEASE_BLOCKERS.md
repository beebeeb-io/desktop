# Release blockers

Each item below must be resolved before a desktop release is cut. While this file lists an item,
the release checks refuse: the release workflow (`.github/workflows/release.yml`) and the macOS
release script (`scripts/release-macos-local.sh`) both fail closed. Each blocker is one list item,
a line that starts with `- `. The change that resolves a blocker removes its item.

- The macOS sign-out flow for Finder saves still waiting to upload shipped in 0.8.12 with 1 of its 12 device checks run (D1 passed; D2–D11 and D10b not run, waived for 0.8.12 only). No later desktop release ships until those cases pass on a signed build (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` §13.4), or the maintainers waive again for that release.
- Task 2179, the macOS main window's follow-ups from the #136 review (snapshot reads never held by a slow storage fetch, busy buttons until the refreshed state shows, the status row from Rust's `sync_phase`, the reader's watchdog and coalesced retries): its two device checks must pass on a signed build in the release-candidate pass before the next desktop release that carries slice 1. With API reads slower than 5 s the window leaves "Loading…" as soon as the first read answers; with Beebeeb turned off in Login Items & Extensions and sync paused, Home shows "Sync is paused" with Resume sync beside the Finder row's turned-off state.

- Shared folders 2237d requires Task 13's signed Finder/Explorer device rung before a compatible desktop release/server enablement: guest/member add, rename, move and parent rename (including descendant working-set updates), trash/restore in the owner's Trash, overwrite/version history, revoke plus move-out/restart, local dirty held edits, and three quiet ticks with zero echo uploads/errors or orphan identities. Tasks 3–6 Rust fixtures exercise existing sync/held-edit interfaces; they do not prove native provider behavior or the remaining 2240–2242 scanner/device integration. Record deployed-client behavior and Windows-native evidence with lead glegion.

