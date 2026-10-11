# Release blockers

Each item below must be resolved before a desktop release is cut. While this file lists an item,
the release checks refuse: the release workflow (`.github/workflows/release.yml`) and the macOS
release script (`scripts/release-macos-local.sh`) both fail closed. Each blocker is one list item,
a line that starts with `- `. The change that resolves a blocker removes its item.

- A sign-out discards Finder saves that are still waiting to upload. Until this is fixed, no desktop release ships the Finder write queue.
