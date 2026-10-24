# Release blockers

Each item below must be resolved before a desktop release is cut. While this file lists an item,
the release checks refuse: the release workflow (`.github/workflows/release.yml`) and the macOS
release script (`scripts/release-macos-local.sh`) both fail closed. Each blocker is one list item,
a line that starts with `- `. The change that resolves a blocker removes its item.

- The macOS sign-out flow for Finder saves still waiting to upload shipped in 0.8.12 with 1 of its 12 device checks run (D1 passed; D2–D11 and D10b not run, waived for 0.8.12 only). No later desktop release ships until those cases pass on a signed build (spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` §13.4), or the maintainers waive again for that release.
