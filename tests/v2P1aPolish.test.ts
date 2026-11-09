// P1a: desktop v2's polish pass, honest states and review follow-ups (desktop #268, #270, #271, #273). Each test names
// the finding or the board frame it pins; each was seen red first (the PR body has both runs).
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { findGoesTo } from '../src/v2/sections/open/find'
import { createKeptCopy, keptCopyName, saveNoteText, tagContent, versionNews } from '../src/v2/sections/open/model'
import { failedWords } from '../src/v2/sections/photos/Dialogs'
import { DriveSourceError } from '@beebeeb/shared/drive-ui'
import { officeSave } from '../src/v2/sections/open/office/save'
import { NOT_INDEXED, saveFailure } from '../src/v2/sections/open/office'
import { isProvisional } from '../src/v2/sections/open'
import { folderKey, shown, videoReadable, type PhotoCell } from '../src/v2/sections/photos/model'
import { READ_CAP_BYTES } from '../src/v2/source/desktop'
import { ThumbCache } from '../src/v2/sections/open/thumbCache'
import { actionFailure } from '../src/v2/sections/files/model'
import { routeFilter } from '../src/v2/sections/sync-history/model'

const src = (path: string) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8')

describe('#273 finding 14: Ctrl+F while editing never leaves the editor for Search (DW-1523 "Find in the text")', () => {
  test('in the text editor and in Office, Ctrl+F stays; elsewhere in Open it is Search', () => {
    expect(findGoesTo('edit', 0)).not.toBe('search')
    expect(findGoesTo('office', 0)).not.toBe('search')
    expect(findGoesTo('new-document', 0)).not.toBe('search')
    expect(findGoesTo(null, 0)).toBe('search')
    expect(findGoesTo(undefined, 0)).toBe('search')
  })
  test('a view with its own find takes it, wherever it is', () => {
    expect(findGoesTo(null, 1)).toBe('own')
    expect(findGoesTo('edit', 1)).toBe('own')
  })
  test('the text editor owns Find and opens its own find panel; Open asks findGoesTo before it goes to Search', () => {
    const open = src('v2/sections/open/index.tsx')
    expect(open).toContain("useOwnFind(true, openFind)")
    expect(open).toContain("findGoesTo(nav.route.sub) === 'search'")
    expect(open).not.toContain('findOwners')
  })
})

describe('#273 findings 3 and 12: the save state is the engine’s, never a time window or a number made in the window (DW-1524)', () => {
  const saving = { base: 3, pending: { kind: 'save' as const, from: 3, content: 'c-mine' } }
  test('until the engine has uploaded the save, it is only saved on this PC', () => {
    expect(versionNews(saving, { version: 3, local: 'downloaded' })).toBe('same')
    expect(versionNews(saving, { version: 4, local: 'syncing', content: 'c-mine' })).toBe('same')
    expect(saveNoteText({ kind: 'local' })).toBe('Saved on this PC · encrypting…')
  })
  test('the upload landing is this window’s save however late it lands, with Rust’s version number', () => {
    expect(versionNews(saving, { version: 4, local: 'downloaded', content: 'c-mine' })).toBe('saved')
    expect(versionNews(saving, { version: 6, local: 'downloaded', content: 'c-mine' })).toBe('saved')
    expect(saveNoteText({ kind: 'encrypted', version: 6 })).toBe('Saved · encrypted · version 6')
  })
  test('another device’s version that lands before the upload is news, not this window’s save', () => {
    expect(versionNews(saving, { version: 4, local: 'downloaded', content: 'c-hers' })).toBe('news')
  })
  test('a restore from the open file lands as a restore, never as another device’s version (finding 8)', () => {
    expect(versionNews({ base: 5, pending: { kind: 'restore', from: 5 } }, { version: 6, local: 'downloaded' })).toBe('restored')
    expect(versionNews({ base: 5, pending: { kind: 'restore', from: 5 } }, { version: 5 })).toBe('same')
  })
  test('the content part of Rust’s tag is what follows the version', () => {
    expect(tagContent('v3-0a1b2c')).toBe('0a1b2c')
    expect(tagContent('v3')).toBeNull()
  })
})

describe('#273 finding 2: "Keep both" picks a free name, in the text editor and in Office', () => {
  test('the label, then the label with a number, skipping names the folder has in any case', () => {
    expect(keptCopyName('Meeting notes.md', 'This PC', [])).toBe('Meeting notes (This PC).md')
    expect(keptCopyName('Meeting notes.md', 'This PC', ['meeting notes (this pc).md'])).toBe('Meeting notes (This PC 2).md')
    expect(keptCopyName('Plan.docx', 'edited on this PC', ['Plan (edited on this PC).docx', 'Plan (edited on this PC 2).docx'])).toBe('Plan (edited on this PC 3).docx')
    expect(keptCopyName('README', 'This PC', [])).toBe('README (This PC)')
  })
  test('a second conflict on the same file makes a second copy; a name Rust refuses moves on to the next', async () => {
    const tried: string[] = []
    const onDisk = new Set(['meeting notes (this pc).md', 'meeting notes (this pc 2).md'])
    const create = async (n: string) => {
      tried.push(n)
      if (onDisk.has(n.toLowerCase())) throw new DriveSourceError('name-taken')
      return { id: `local:${n}` }
    }
    // The listing shows the first copy only; Rust knows the second (cloud-only).
    const r = await createKeptCopy('Meeting notes.md', 'This PC', async () => ['Meeting notes.md', 'Meeting notes (This PC).md'], create)
    expect(r.name).toBe('Meeting notes (This PC 3).md')
    expect(tried).toEqual(['Meeting notes (This PC 2).md', 'Meeting notes (This PC 3).md'])
  })
  test('any other refusal stops at once, and a taken name has its own words', async () => {
    let calls = 0
    const create = async () => { calls += 1; throw new DriveSourceError('read-only') }
    expect(createKeptCopy('a.md', 'This PC', async () => [], create)).rejects.toThrow()
    await Promise.resolve()
    expect(calls).toBe(1)
    expect(failedWords(new DriveSourceError('name-taken'), 'save your edit')).toBe('Couldn’t save your edit. There’s already a file with that name here.')
  })
  test('both editors use it', () => {
    expect(src('v2/sections/open/index.tsx')).toContain("createKeptCopy(item.name, 'This PC'")
    expect(src('v2/sections/open/office/index.tsx')).toContain("createKeptCopy(name, 'edited on this PC'")
  })
})

describe('#273 finding 11: a new Office document’s first save is done once Rust has made the file', () => {
  const made = { id: 'local:Documents/Plan.docx', parentId: 'f-documents', name: 'Plan.docx', isFolder: false, size: 0, modifiedAt: '', shared: { kind: 'none' }, local: 'syncing', hasThumbnail: false } as const
  const notIndexed = async () => { throw new DriveSourceError('not-found') }
  test('the engine not having indexed the new file yet is no failure: the save made it, with its tag to come', async () => {
    const r = await officeSave({ creating: true, fileId: null, doc: null }, new Uint8Array([1]), {
      make: async () => made, tagOf: async () => null, saveBytes: async () => ({ versionId: 'never' }),
    })
    expect(r).toEqual({ kind: 'made', item: made, versionId: null })
  })
  test('a tag read that throws is no failure either', async () => {
    const r = await officeSave({ creating: true, fileId: null, doc: null }, new Uint8Array([1]), { make: async () => made, tagOf: notIndexed, saveBytes: async () => ({ versionId: 'never' }) })
    expect(r.kind).toBe('made')
  })
  test('the next save without a tag yet writes nothing and says so, never "Saved"', async () => {
    const wrote: string[] = []
    const r = await officeSave({ creating: false, fileId: made.id, doc: { versionId: null } }, new Uint8Array([2]), {
      make: async () => made, tagOf: async () => null, saveBytes: async (id) => { wrote.push(id); return { versionId: 'v0-x' } },
    })
    expect([r.kind, wrote.length]).toEqual(['not-indexed', 0])
    expect(NOT_INDEXED).toContain('Not saved yet')
  })
  test('once indexed, the next save writes over the tag the engine gave', async () => {
    const r = await officeSave({ creating: false, fileId: made.id, doc: { versionId: null } }, new Uint8Array([2]), {
      make: async () => made, tagOf: async () => 'v0-abc', saveBytes: async (_id, _b, base) => ({ versionId: `${base}>v0-def` }),
    })
    expect(r).toEqual({ kind: 'saved', versionId: 'v0-abc>v0-def' })
  })
  test('a save with nothing to save into is an error, never a success', async () => {
    await expect(officeSave({ creating: false, fileId: null, doc: null }, new Uint8Array(), { make: async () => made, tagOf: async () => 'x', saveBytes: async () => ({ versionId: 'y' }) })).rejects.toThrow()
  })
  test('a taken name has its own words', () => {
    expect(saveFailure(new DriveSourceError('name-taken'))).toBe('Couldn’t save it. Your edit is still here. There’s already a file with that name here.')
    expect(saveFailure(new DriveSourceError('failed'))).toBe('Not saved. Your edit is still here; try again.')
  })
})

describe('#273 finding 8: a restore from the open file reloads it when it lands, as a restore', () => {
  test('Version history over the open file tells the file, which reloads and says it is the restore', () => {
    const open = src('v2/sections/open/index.tsx')
    expect(open).toContain('onRestored={watch.restoredHere}')
    expect(open).toMatch(/restored: \(version\) => \{[\s\S]{0,300}Version restored\. This is now version \$\{version\}[\s\S]{0,200}retry\(\)/)
    const hosts = src('v2/sections/open/hosts.tsx')
    expect(hosts).toContain('if (onRestored) { onRestored();')
  })
})

describe('#273 finding 11 (New file): a file this window just made opens once the engine has indexed it', () => {
  test('a provisional id waits for the engine’s event instead of reading as a file that cannot be decrypted', () => {
    expect(isProvisional('local:Documents/Untitled.md')).toBe(true)
    expect(isProvisional('f-documents')).toBe(false)
    const open = src('v2/sections/open/index.tsx')
    expect(open).toContain("if (isProvisional(id) && e instanceof DriveSourceError && e.code === 'not-found') return setUnindexed(true)")
    expect(open).toMatch(/if \(!unindexed\) return\n\s+return source\.subscribe\(\(e\) => \{ if \(e\.kind === 'changed' \|\| e\.kind === 'item'\) setTries/)
  })
})

describe('#273 finding 1: until the page view is drawn in the window, a PDF opens in another app, never a refused frame', () => {
  test('Open frames no decrypted content, and a PDF gets the open-in-another-app state with its button', () => {
    const open = src('v2/sections/open/index.tsx')
    expect(open).not.toContain('<iframe')
    expect(open).toContain("useBlobUrl(kind === 'pdf' ? null : bytes")
    expect(open).toContain(`{kind === 'pdf' && <><ToolbarButton label="Open in another app…" onClick={openElsewhere} />`)
    expect(open).not.toMatch(/else if \(kind === 'pdf'\) body =/)
  })
})

describe('#273 finding 6: switching off the photos directly in Files hides them (DW-0604)', () => {
  const cell = (id: string, parentId: string | null): PhotoCell => ({ entry: { item: { id, parentId, name: `${id}.jpg`, isFolder: false, size: 1, modifiedAt: '2026-11-01T10:00:00', shared: { kind: 'none' }, local: 'online-only', hasThumbnail: true } } }) as PhotoCell
  test('the root folder has its own key, so its switch hides its photos and nothing else', () => {
    const cells = [cell('a', null), cell('b', null), cell('c', 'f-lisbon')]
    expect(shown(cells, new Set([folderKey(null)])).map((c) => c.entry.item.id)).toEqual(['c'])
    expect(shown(cells, new Set([folderKey('f-lisbon')])).map((c) => c.entry.item.id)).toEqual(['a', 'b'])
    expect(folderKey(undefined)).toBe(folderKey(null))
    expect(folderKey('f-lisbon')).toBe('folder:f-lisbon')
  })
  test('the dialog’s choice is saved for the root too, and read back as off', () => {
    const photos = src('v2/sections/photos/index.tsx')
    expect(photos).toContain('if (!x.on) next.add(folderKey(x.id))')
    expect(photos).toContain('on: !hidden.has(folderKey(f.id))')
  })
})

describe('#273 finding 7: a video the window cannot read is said so before any byte is read (DW-0611)', () => {
  const GB = 1024 ** 3
  test('over Rust’s read cap it is too large; a video not on this PC needs its size free; else it plays', () => {
    expect(videoReadable({ size: 4 * GB, local: 'online-only' }, 100 * GB, READ_CAP_BYTES)).toBe('too-large')
    expect(videoReadable({ size: 200 * 1024 ** 2, local: 'online-only' }, 100 * 1024 ** 2, READ_CAP_BYTES)).toBe('no-room')
    expect(videoReadable({ size: 200 * 1024 ** 2, local: 'downloaded' }, 100 * 1024 ** 2, READ_CAP_BYTES)).toBe('ok')
    expect(videoReadable({ size: 200 * 1024 ** 2, local: 'online-only' }, undefined, READ_CAP_BYTES)).toBe('ok')
  })
  test('the window’s cap is Rust’s (fsops::BYTES_LIMIT)', () => {
    const fsops = readFileSync(new URL('../src-tauri/src/drive_v2/fsops.rs', import.meta.url), 'utf8')
    const m = /pub\(crate\) const BYTES_LIMIT: u64 = ([\d *]+);/.exec(fsops)
    expect(m).not.toBeNull()
    expect(m![1].split('*').reduce((n, x) => n * Number(x.trim()), 1)).toBe(READ_CAP_BYTES)
  })
  test('the player checks first, and keeps only the Blob', () => {
    const viewer = src('v2/sections/photos/Viewer.tsx')
    expect(viewer).toMatch(/videoReadable\(\{ size, local \}, free, READ_CAP_BYTES\)[\s\S]{0,300}source\.readBytes\(id\)/)
  })
})

describe('#273 finding 4: the RAW strip frees a thumbnail only when it is replaced or pushed out, never while shown', () => {
  test('a later run that needs the same thumbnails reads them from the cache and frees none', () => {
    const freed: string[] = []
    const c = new ThumbCache(4, (u) => freed.push(u))
    c.put('a', 'large', 'blob:A'); c.put('b', 'small', 'blob:B')
    // The siblings arrive and the strip's set changes: the same items are read back, nothing is freed.
    expect([c.get('a', 'large'), c.get('a', 'small'), c.get('b', 'small'), c.get('c', 'small')]).toEqual(['blob:A', 'blob:A', 'blob:B', undefined])
    expect(freed).toEqual([])
  })
  test('a small thumbnail never replaces a large one; a replaced URL and the least recently used are freed', () => {
    const freed: string[] = []
    const c = new ThumbCache(2, (u) => freed.push(u))
    c.put('a', 'large', 'blob:A')
    c.put('a', 'small', 'blob:a')
    expect([c.get('a', 'large'), freed]).toEqual(['blob:A', ['blob:a']])
    expect(c.get('b', 'large')).toBeUndefined()
    c.put('b', 'small', 'blob:b1'); c.put('b', 'large', 'blob:B')
    expect(freed).toEqual(['blob:a', 'blob:b1'])
    c.get('a', 'small') // a is used again, so b is the least recently used
    c.put('c', 'small', 'blob:C')
    expect([freed, c.size, c.get('b', 'small')]).toEqual([['blob:a', 'blob:b1', 'blob:B'], 2, undefined])
  })
  test('the strip keeps its URLs across runs: no cleanup revokes them', () => {
    const open = src('v2/sections/open/index.tsx')
    expect(open).toContain('const cache = rawThumbCache(source)')
    expect(open).not.toContain('made.forEach((u) => URL.revokeObjectURL(u))')
  })
})

describe('#273 findings 5 and 9: opening a file reads only what it needs, and a stale read never lands', () => {
  const open = () => src('v2/sections/open/index.tsx')
  test('the free space is read only for a file not on this PC, and the too-large view takes that figure', () => {
    expect(open()).toMatch(/if \(item\.local === 'online-only'\) \{\n\s+const free = await extras\.status\(\)/)
    expect(open()).not.toContain('Promise.all([source.details(id), extras.status()')
    expect(open()).toContain('known={phase.free}')
    expect(open()).toContain('if (known === undefined) extras.status()')
  })
  test('a run that a newer one replaced stops after its second details read', () => {
    expect(open()).toMatch(/const \{ item \} = await source\.details\(id\)\.catch\(\(\) => \(\{ item: null \}\)\)\n\s+if \(!live\) return\n\s+if \(item\) return setPhase\(\{ name: 'too-large', item \}\)/)
  })
})

describe('#268 review: failures say what failed, the Problems filter follows the route, one close function', () => {
  test('a failed New folder names the action and offers Try again, which reopens the dialog', () => {
    const fv = src('v2/sections/files/FolderView.tsx')
    expect(fv).toContain("fail(e, undefined, { action: 'create the folder', again: openNewFolder })")
    expect(actionFailure('create the folder', 'offline')).toEqual({ text: 'Couldn’t create the folder. Check your connection.', retry: true })
    expect(fv).not.toMatch(/fail\(e\)\s*\}/)
  })
  test('Ctrl+Z’s failed undo is the toast’s: what failed, and Try again with the same token', () => {
    const fv = src('v2/sections/files/FolderView.tsx')
    expect(fv).toMatch(/useCommand\('undo', useCallback\(\(\) => \{[\s\S]{0,200}putBack\(token\)/)
    expect(fv).not.toContain('source.undoTrash(token).catch((e: unknown) => fail(e))')
  })
  test('Problems from the banner, All again once the route no longer asks for it', () => {
    expect(routeFilter('problems')).toBe('problems')
    expect(routeFilter(null)).toBe('all')
    expect(routeFilter(undefined)).toBe('all')
    expect(src('v2/sections/sync-history/index.tsx')).toContain('useEffect(() => { setFilter(routeFilter(nav.route.sub)) }, [nav.route])')
  })
  test('the uploads panel’s close is one memoised function', () => {
    const live = src('v2/sections/files/liveExtras.tsx')
    expect(live).toContain('const closeUploads = useCallback(() => setUploadsOpen(false), [])')
    expect(live).not.toContain('closeUploads: () =>')
  })
})
