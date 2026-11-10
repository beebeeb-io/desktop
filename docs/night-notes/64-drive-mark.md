# 64 — The sidebar shows Beebeeb Drive

Date: 2026-11-10 (night wave 3)

## What changed

- `src/v2/chrome/Sidebar.tsx` (brand row only): the top of the sidebar now draws
  `BBProductLockup product="drive" size={24}` (the board's `MacSide`, `.mk-side-brand`)
  and, under it in the same row, a separate text node `Beebeeb Drive`, above the Files
  link. The row is `px-2.5`, matching the nav links. The lockup's own amber on `.io` is
  the product mark; the bar stays free of `amber`. `storageLine`, `storageFigure`,
  `rowCount` and the meter block are untouched.
- `src/v2/V2App.tsx`: `windowTitle(section)` is exported beside `TITLES` — Files is
  `Beebeeb Drive`, every other section is `${TITLES[section]} - Beebeeb Drive`. The
  caption expression in `V2Shell` is the same expression, kept inline (see the deviation
  below). The account effect, the storage effect, the counts and the `Sidebar` props are
  untouched.
- `src/v2/frames/b04.tsx`: the DW-0401 window's caption string is `Beebeeb Drive`.
  `accountName="Anna"` and the frame's storage figures are unchanged.
- `tests/driveMark.test.tsx` (new): three tests — the brand row's lockup at 24 px plus the
  separate `Beebeeb Drive` text node above Files; `windowTitle('files')`; and
  `windowTitle('settings')`.

## Deviation: `windowTitle` is exported but the caption expression stays inline

The brief asked to use `windowTitle` at the caption line. The component harness
(`tests/fixtures/componentHarness.ts`) executes the `V2Shell` function declaration with
an explicit binding list; a module-level function is not in that scope, so calling
`windowTitle` there throws `ReferenceError: windowTitle is not defined` and breaks
`tests/storageMeter.test.tsx` and `tests/sidebarAccountName.test.tsx`, which must stay
unchanged. The caption therefore keeps the identical expression inline, and
`windowTitle` is the exported pure form the test pins. Both read the same `TITLES` map;
no second map was added.

## Red first, then green

Red (Sidebar reverted to HEAD, `windowTitle` returning the old strings):

```
date: 2026-11-10T23:29:11Z
command: bun test tests/driveMark.test.tsx
Expected: "drive"  Received: undefined
(fail) the sidebar shows Beebeeb Drive > the brand row carries the drive lockup at 24 px and the words Beebeeb Drive above Files
Expected: "Beebeeb Drive"  Received: "Beebeeb"
(fail) the window caption names Beebeeb Drive > Files is Beebeeb Drive
Expected: "Settings - Beebeeb Drive"  Received: "Settings - Beebeeb"
(fail) the window caption names Beebeeb Drive > Settings is Settings - Beebeeb Drive
 0 pass
 3 fail
```

Green:

```
date: 2026-11-10T23:29:20Z
command: bun test tests/driveMark.test.tsx
 3 pass
 0 fail
```

The two shell tests, each in its own process:

```
date: 2026-11-10T23:29:20Z  bun test tests/storageMeter.test.tsx       4 pass / 0 fail
date: 2026-11-10T23:29:21Z  bun test tests/sidebarAccountName.test.tsx 4 pass / 0 fail
```

`bunx tsc --noEmit` → 0 errors (exit 0, 2026-11-10T23:29:21Z).

The full `bun test` run in this sandbox is 2388 pass / 309 fail; the same 309 fail with
`tests/driveMark.test.tsx` moved aside (2388 pass / 309 fail, 2694 tests), so they are the
sandbox baseline (Finder setup, speed test, trash, sign-in), not this slice.

## Walkthrough

Dev server on `:5251`, headless Chromium at 1280×800, board frames:

- DW-0401 caption `Beebeeb Drive`; brand row text `beebeeb.io DRIVE Beebeeb Drive`; the
  lockup wordmark and `Drive` label each once; the words once; the brand row above Files.
- The meter is one `role="meter"`, no amber class, text `258 GB of 1.1 TB` (the frame's
  account bytes).
- DW-0701 caption `Settings - Beebeeb Drive`.
- 0 page errors.

Screenshots: `01-files.png`, `02-settings.png`.

Evidence (absolute paths, outside the repo):
`/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/drive-mark/`
(`red.log`, `green.log`, `storageMeter.log`, `sidebarAccountName.log`, `tsc.log`,
`walkthrough.log`, `walkthrough.mjs`, `01-files.png`, `02-settings.png`), each log
starting with its `date -u` line.
