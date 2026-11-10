# 56 — Sign out finishes or says why

Date: 2026-11-10 (night wave 3, review round 1)

## What was checked

The slice's test was already green on the first run of this round, so no product
code was changed. The two v2 sign-outs named in the brief are:

- `SignOutDialog.signOut` in `src/v2/sections/settings/account/SecurityPane.tsx`
- `NoPlanWindow.signOut` in `src/v2/auth/Gate.tsx`

Both call `signOutUnknownFiles()` first. A count above zero shows
`SignOutUnknownFilesDialog` (the words are `unknownFilesSignOutCopy`'s, shared with
the legacy window) and calls nothing; Keep copies passes
`{ unknownFiles: 'keep', keptFolder }`, Delete passes `{ unknownFiles: 'delete' }`,
Cancel calls nothing. A count of zero calls `clearSession()` once with no
`unknownFiles`. A failed `clearSession` keeps the question open with `r.reason` and
leaves its buttons enabled.

## Evidence

```
date: 2026-11-10T14:07:19+01:00
command: bun test tests/signOutAsks.test.tsx
 5 pass
 0 fail
 15 expect() calls
Ran 5 tests across 1 file.
```

`bunx tsc --noEmit` → 0 errors.

Red-first and walkthrough evidence from the implementing commit:
`/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/sign-out-asks/`
(`red.txt`, `green.txt`, `tsc.txt`, `walkthrough.txt`, `walk-*.png`).

## Not done

The operation queue purge inside `clear_session_impl` and the Cloud Files teardown
were not touched; the wait this slice removes is the keep-copy inside
`clear_session`, which now runs only after the person chooses.
