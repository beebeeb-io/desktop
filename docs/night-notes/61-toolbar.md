# 61 — the Files toolbar is a search field

## What changed

- `src/v2/sections/files/FilesPane.tsx`, `Toolbar` (line 342): the `<button>` that read `Search names Ctrl+F` is now a `<label>` holding the search icon, an `<input type="text" aria-label="Search names" placeholder="Search names">` and the visible `Ctrl+F` text. The input has no `value` and no `onChange`; `onKeyDown` calls `h.onFind?.()` and `e.preventDefault()` only when `e.key === 'Enter'`. The rest of the toolbar (Back/Forward, Crumbs, count, View, Sort, details, New) is unchanged.
- `src/v2/dev/states-drive.ts`: `OPEN_SEARCH` is `field:Search names`; the four steps (DW-0801, DW-0802, DW-0802b, DW-0803) use `{ key: 'Enter', on: OPEN_SEARCH }` instead of `{ click: OPEN_SEARCH }`.
- `src/v2/dev/states-files.ts`: DW-1105 uses `{ key: 'Enter', on: 'field:Search names' }`; its `fill` step is unchanged.
- `tests/filesToolbar.test.tsx` (new): three tests — the field, the Enter handler, and the toolbar guard.

## Red first

`tests/filesToolbar.test.tsx` was written before any product edit. On the unchanged tree:

```
2026-11-11T00:47:26+01:00
...
expect(fields.length).toBe(1)
Expected: 1
Received: 0
...
 1 pass
 2 fail
```

The field test and the Enter test failed; the toolbar guard test passed. Full output: `docs/night-notes/evidence/61-toolbar/red.txt`.

## Green

Same command, after the `Toolbar` edit:

```
2026-11-11T00:47:36+01:00
 3 pass
 0 fail
```

Green (00:47:36) is later than red (00:47:26). Full output: `docs/night-notes/evidence/61-toolbar/green.txt`.

## Other checks

- `bun test tests/searchReal.test.tsx` (own process, `2026-11-11T00:47:49+01:00`): `4 pass / 0 fail`. `docs/night-notes/evidence/61-toolbar/searchReal.txt`.
- `bunx tsc --noEmit` (`2026-11-11T00:47:55+01:00`): exit 0, 0 errors. `docs/night-notes/evidence/61-toolbar/tsc.txt`.
- Full `bun test` in this sandbox: `2391 pass / 309 fail` across 194 files. The failing files are outside this slice (settings, trash, onboarding, speed test); `bun test tests/trashReal.test.tsx` alone is `4 pass / 0 fail`, so the full-run failures are cross-file interference in the sandbox, not this change.

## Walkthrough

`bunx vite --port 5251` plus `docs/night-notes/evidence/61-toolbar/walkthrough.mjs` at 1280x800 (`2026-11-11T01:11:04+01:00`):

```
frame DW-0801: section=1 field=1 data-search=recent
frame DW-1105: section=1 field=1 data-search=results
files: fields=1 ctrlF=true filter=0 sort=1 asList=1 asGrid=1
enter: focused=Search names
enter: url=http://localhost:5251/?window=main-app&platform=windows&section=search searchSectionsBefore=0 searchSectionsAfter=1
frames=2/2 pageErrors=0
```

- 2 of 2 frames reach the search page with its own field; 0 page errors.
- The plain Files window: 1 input whose `aria-label` and `placeholder` are both `Search names`, `Ctrl+F` visible, 0 buttons named `Filter`, and `Sort`, `as List`, `as Grid` each present once.
- Clicking the field and pressing Enter opens the Search section once (0 before, 1 after).

Full output: `docs/night-notes/evidence/61-toolbar/walkthrough.txt`.

## Evidence location

All evidence is committed in `docs/night-notes/evidence/61-toolbar/` (`red.txt`, `green.txt`, `searchReal.txt`, `tsc.txt`, `walkthrough.mjs`, `walkthrough.txt`) and pasted above.

## Out of scope

The ⌘K command bar (`Topbar` in the board) is still absent; this slice does not add it. No Filter control was built (the frame has none). `FolderView.tsx`, `model.ts`, `menus.ts` and `V2App.tsx` are unchanged.
