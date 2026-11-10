# 63 — Preview shows the file's own text

Date: 2026-11-10 (night wave 3)

## What was checked

`PreviewHost` in `src/v2/sections/files/FolderView.tsx` (around line 530) is the live
Files preview. Its text branch (line 543) calls `source.readText(item.id)` and sets
`{ kind: 'text', text: d.text }`; the text body is the `<pre>` at the end of the host
(line 565) inside `PreviewPanel`, whose dialog is named `Preview <name>`. A PDF is forced
to `{ kind: 'none' }` (line 542) and a rejected read also lands on `{ kind: 'none' }`
(line 543), which draws `NoPreview` — the honest state, never a blank panel.

`tests/previewRealFile.test.tsx` mounts that host with a plain source double (no
fixtures, no `src/v2/dev`) and asserts two things:

- (a) `quarterly.txt` whose `readText` returns `quarterly line 1` renders that sentence
  in the `<pre>` inside the dialog named `Preview quarterly.txt`, and `readText` was
  called once with `i-quarterly`.
- (b) a text item whose `readText` rejects draws `NoPreview` ("No preview for this kind
  of file") and no `<pre>` body — never a blank panel and never another file's text.

## Red first, then green

The guard is green on the real code, so it was seen to fail on a one-line mutant: the
`isText` expression in `PreviewHost` was replaced with `false` (dropping the
`source.readText` branch).

```
date: 2026-11-10T23:31:36+01:00
command: bun test tests/previewRealFile.test.tsx
(mutant: const isText = false)
Expected: "quarterly line 1"
Received: ""
(fail) Space on quarterly.txt names that file and draws its own text, reading it once
(fail) a text file whose read fails says there is no preview, never a blank panel
 0 pass
 2 fail
```

The line was restored, then:

```
date: 2026-11-10T23:31:48+01:00
command: bun test tests/previewRealFile.test.tsx
 2 pass
 0 fail
```

The PDF path is unchanged; `tests/pdfPreview.test.tsx` in its own process:

```
date: 2026-11-10T23:31:48+01:00
command: bun test tests/pdfPreview.test.tsx
 2 pass
 0 fail
```

`bunx tsc --noEmit` → 0 errors.

## Where the installed app still waits

The installed app's empty Files list is `src-tauri/src/owner_keys.rs` (`prepare_owned_rows`
around line 729, `stored_name` around line 263) — slice 65, not this one. Until that layer
is fixed there is no file row to select, so the walkthrough waits on slice 65.

Evidence (absolute paths, outside the repo):
`/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/preview-file/`
(`green-first-run.txt`, `mutant-red.txt`, `restored-green.txt`, `pdf-green.txt`, `tsc.txt`).
