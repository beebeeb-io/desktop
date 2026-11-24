# a11y: UI Automation walk

`uia-walk.ps1` (Windows PowerShell 5.1, no install) lists interactive elements in the running app that a
screen reader cannot name: keyboard-focusable or Invoke/Toggle/SelectionItem/ExpandCollapse, with an empty
Name or ControlType Custom/Pane. Output: JSON, a `WINDOW` count line per window, `UIA-WALK: <n> unnamed
controls`. Exit 1 if any, 0 if none, 2 if no window was found.

## Run
Self-test (no app): `powershell -NoProfile -File uia-walk.ps1 -SelfTest`

On the Windows test machine the walk must run inside the logged-in desktop session (UIA sees no windows from a service session):

    scripts/coord/remote-build.sh win desktop <branch> --dir native/beebeeb-drive-rune/scripts/a11y -- \
      'powershell -NoProfile -File $env:BB_RUN_IN_DESKTOP -Script uia-walk.ps1'

Per window (the 3/3 real check: Files, Photos, Settings; open each view first):
`.\uia-walk.ps1 -Process beebeeb-drive -Window Files -Out files.json` (likewise Photos, Settings).
Adjust `-Process` to the installed exe name if it differs.

Keyboard pass: add `-Keys` (`-KeyRepeat 12`). It sends Tab, F6, Down, Right and records the focused element
after each key under `KeyPass` (`Unnamed: true` = focus landed on an unnamed control). Look for focus that
never moves, leaves the window, or repeats a short loop.

## Final-run checklist (backlog 1656)
Repeat the walk plus `-Keys` on Files, Photos, Settings under each:
- DPI 100 % and 150 % (Settings > Display > Scale; sign out/in): no clipped or overlapping controls, same counts.
- Second monitor: drag the window across, mixed DPI if available; no blur, focus rings still visible.
- Regional formats: e.g. nl-NL and en-US (date, number, decimal comma): sizes and dates render and read correctly.
