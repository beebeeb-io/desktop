# Release blockers

Each item below must be resolved before a desktop release is cut. While this file lists an item,
the release checks refuse: the release workflow (`.github/workflows/release-native-windows.yml`; on the
0.8.x line also `release.yml` and the macOS release script) fails closed. Each blocker is one list item,
a line that starts with `- `. The change that resolves a blocker removes its item. An item prefixed
`- [macOS] ` (exact token, case-sensitive) is scoped to macOS only and is skipped by the Windows
and Linux release workflow; any other form, including `[macos]` or `[MacOS]`, is unscoped and
blocks every platform (fail-closed).

No item blocks a release at present.

## Waived

A waived item is not a list item, so the release checks do not read it. Each entry keeps the item's text,
the date and the maintainer's words.

Waived 2026-11-28 by the maintainer (desktop wave 40, PR #489): "that check is not needed". The three
[macOS] items below no longer block a macOS release.

Waived [macOS] item 1: The macOS sign-out flow for Finder saves still waiting to upload shipped in 0.8.12 with 1 of its 12 device checks run (D1 passed; D2–D11 and D10b not run, waived for 0.8.12 only). No later desktop release ships until those cases pass on a signed build (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` §13.4), or the maintainers waive again for that release.

Waived [macOS] item 2: Task 1683 slice 3, the macOS menu-bar popover (#160): its "Review…" button opens Home, which shows no conflicts, so the popover's Review… → conflicts view needs #161 (slice 4, Review and Trash). No desktop release that carries the popover ships until #161 lands, or until the button is hidden.

Waived [macOS] item 3: Task 2179, the macOS main window's follow-ups from the #136 review (snapshot reads never held by a slow storage fetch, busy buttons until the refreshed state shows, the status row from Rust's `sync_phase`, the reader's watchdog and coalesced retries): its two device checks must pass on a signed build in the release-candidate pass before the next desktop release that carries slice 1. With API reads slower than 5 s the window leaves "Loading…" as soon as the first read answers; with Beebeeb turned off in Login Items & Extensions and sync paused, Home shows "Sync is paused" with Resume sync beside the Finder row's turned-off state.
