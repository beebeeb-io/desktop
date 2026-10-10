# 2026-09-30 — macOS menu-bar popover redesign (task 1683)

**Status:** design ruling APPROVED. Guus accepted **all seven recommendations** on 2026-10-01.
**Hi-fi source:** [`design/hifi/macos-menubar-popover.html`](../../design/hifi/macos-menubar-popover.html) (authored by the Claude Cloud design lane, 2026-09-30, as the task-1683 design review; recovered from `~/Downloads` and committed here 2026-10-01 — it was never committed at design time).
**Implementation plan:** nine slices, each ships tests that are seen failing first; the last rung is a manual pass by Guus on the next macOS alpha. Slice 4 (the macOS Settings window, 4 tabs) already shipped via PR #85 + notes amendment #86.
**Amended 2026-10-06 (ruling R5, [spec A](2026-10-06-macos-finder-setup-reconciler.md) §12):** the "finder not added" state and its "Add to Finder" action are removed: Beebeeb adds itself to Finder after sign-in. "Finder add failed" shows one sentence and one action per reason (spec A §6.2), and "Try again" survives only inside a failure. The `States:` bullet below keeps its original wording for the record.

## Popover shape
- 372×488 pt popover anchored under the menu-bar icon.
- Placement: under the icon, on the icon's display. Never centres on screen, never remembers position.
- States: up to date / syncing / vault locked / paused / error / storage full / signed out / finder not added / finder add failed / dark.
- Gear menu: Settings… / Pause sync / Help / Quit.
- Settings shrinks to 4 tabs: General / Account / Sync / About (shipped, slice 4).
- Fewer windows: 6→4 window kinds, 4→1 idle webviews; conflicts no longer auto-open windows.

## The seven choices (all answers = the recommendation)
1. **At login** — show nothing at login; open the popover once after first install only.
2. **Dock icon** — none normally; show while Settings, onboarding, or review is open.
3. **Translucency** — opaque. Public APIs only; Proton-style translucency needs private APIs that block Mac App Store distribution.
4. **Quick Search** — cut from the main surface; version history stays in the single review window.
5. **Third footer action** — Add storage (elsewhere Pause/Resume).
6. **Settings window placement** — top-right under the menu bar, on the icon's display; onboarding stays centred (860×640).
7. **Errors** — keep the rule "toast for transient, inline for blocking"; fix only the double render bug.

## Follow-ups spun off from the 0.8.7-alpha hand test (2026-10-01)
- 1694 (in-development, PR #87): Finder blocked adding files to folders — missing add-subitems capability bit; fixed RED-first.
- 1695 (backlog): hydrate progress not visible when opening cloud-only files from Finder.
- 1696 (backlog): stale FileProvider domains (`io.beebeeb.desktop.FileProvider`, `Beebeeb-Drive` volume) need signed-runtime cleanup.