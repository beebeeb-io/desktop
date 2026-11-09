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
