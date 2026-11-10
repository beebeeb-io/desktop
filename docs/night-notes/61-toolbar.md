# 61 — Sort and search match the Files frame

Date: 2026-11-10 (night wave 3; evidence re-captured 2026-11-11)

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

- `red.txt` — `bun test tests/filesToolbar.test.tsx` against the pre-change
  `FilesPane.tsx` (HEAD~2), captured `Wed Nov 11 12:05:59 AM CET 2026`:
  1 pass / 2 fail. The failing assertion:

  ```
  expect(fields).toHaveLength(1)
  Expected length: 1
  Received length: 0
  ```

  The toolbar control was a `button` whose text is `Search names`, so the field
  count was 0 and Enter had no field to reach (`onFind` called 0 times).
- `green.txt` — after: 3 pass / 0 fail (`Wed Nov 11 12:05:24 AM CET 2026`).
- `searchReal.txt` — `bun test tests/searchReal.test.tsx`: 4 pass / 0 fail.
- `tsc.txt` — `bunx tsc --noEmit`: exit 0.
- `walkthrough.txt` + screenshots (`files-toolbar.png`, `sort-menu.png`,
  `search-after-enter.png`): 18/18 checks — field, Ctrl+F, no Filter, Enter
  opens search, sort menu labels.

The "no button named Search names" assertion matches a button whose visible
text contains `Search names` (the old control read `Search names Ctrl+F`), so
it fails if the old control survives; verified against the pre-change tree
(1 button found).

## Still absent

The separate command field (the board's Topbar, ⌘K) is not this slice and is
not built.
