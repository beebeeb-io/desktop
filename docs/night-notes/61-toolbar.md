# 61 — Sort and search match the Files frame

Date: 2026-11-10 (night wave 3)

## What changed

`Toolbar` in `src/v2/sections/files/FilesPane.tsx` now draws the board's
`MacToolbar` search: one `input` with accessible name and placeholder
`Search names`, the visible shortcut `Ctrl+F` beside it, and Enter calling the
existing `onFind` (which `FolderView` points at the search section). A letter
does not call `onFind`. The old `button` whose text was `Search names` is gone,
so the dev recipes in `states-drive.ts` and `states-files.ts` press Enter on
`field:Search names` instead of clicking `btn:Search names`.

Sort, `as List` and `as Grid` stay. There is no Filter control, matching the
frame.

## Evidence

`/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/files-toolbar/`

- `red.txt` — `bun test tests/filesToolbar.test.tsx` before the change:
  1 pass / 2 fail (0 fields named `Search names`; Enter reached no field).
- `green.txt` — after: 3 pass / 0 fail.
- `searchReal.txt` — `bun test tests/searchReal.test.tsx`: 4 pass / 0 fail.
- `tsc.txt` — `bunx tsc --noEmit`: exit 0.
- `walkthrough.mjs` + screenshots: field, Ctrl+F, no Filter, Enter opens
  search, sort menu labels.

## Still absent

The separate command field (the board's Topbar, ⌘K) is not this slice and is
not built.
