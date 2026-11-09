// P1c: Settings › Today's four panes read only what the app can answer, never the board's example data. The folder tree
// maps `list_vault_folders` + `get_selective_sync` onto the dialog's model; a path's kind and section stay as #274 fixed
// them; and the four production files reach into the fixtures 0 times.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { VaultItem } from '../src/desktopApi'
import { cutPath, kindOf, sectionOfPath } from '../src/v2/sections/settings/today/insights'
import { keepTreeFrom } from '../src/v2/sections/settings/today/keep'

const file = (name: string) => readFileSync(new URL(`../src/v2/sections/settings/today/${name}`, import.meta.url), 'utf8')
const hits = (src: string) => src.split('source/fixtures').length - 1

const folder = (id: string, name: string, size_bytes: number, children: VaultItem[] = []): VaultItem => ({
  id, name, is_folder: true, excluded: false, size_bytes, file_count: children.length, on_disk_bytes: 0, children,
})

describe('keepTreeFrom: the folder dialog reads its tree and checked set from the app’s own answers', () => {
  test('no folders, nothing excluded: 0 nodes and 0 checked', () => {
    const { tree, checked } = keepTreeFrom([], [])
    expect(tree.length).toBe(0)
    expect(checked.size).toBe(0)
  })

  test('3 folders, one excluded: sizes in GB, the excluded subtree unchecked, the kept ones checked', () => {
    const folders = [
      folder('a', 'Alpha', 2_000_000_000),
      folder('b', 'Beta', 3_500_000_000, [folder('b1', 'One', 1_000_000_000), folder('b2', 'Two', 2_000_000_000)]),
      folder('c', 'Gamma', 4_000_000_000, [folder('c1', 'Three', 1_500_000_000), folder('c2', 'Four', 2_500_000_000)]),
    ]
    const { tree, checked } = keepTreeFrom(folders, ['b'])
    expect(tree.length).toBe(3)
    expect(tree[0].gb).toBeCloseTo(2)
    expect(tree[1].gb).toBeCloseTo(3.5)
    expect(tree[1].children?.length).toBe(2)
    expect(tree[2].children?.length).toBe(2)
    expect(checked.has('a')).toBe(true)
    expect(checked.has('b')).toBe(false)
    expect(checked.has('b1')).toBe(false)
    expect(checked.has('b2')).toBe(false)
    expect(checked.has('c')).toBe(true)
  })
})

describe('Storage insights keeps the kind and section the #274 review fixed', () => {
  test('a .mov in Backups is a Video in the backups section; a leading slash is not shown', () => {
    expect(kindOf('x/Backups/a.mov', 'media')).toBe('Video')
    expect(sectionOfPath('/Backups/Anna-PC/Videos/Render final.mov')).toBe('backups')
    expect(sectionOfPath('\\Backups\\Anna-PC\\clip.mov')).toBe('backups')
    expect(sectionOfPath('/docs/report.pdf')).toBe('files')
    expect(cutPath('/docs/report.pdf')).toBe('docs/report.pdf')
  })
})

describe('the four Today panes reach into the board’s fixtures 0 times', () => {
  test('about, insights, keep and location name no source/fixtures import', () => {
    for (const name of ['about.tsx', 'insights.tsx', 'keep.tsx', 'location.tsx']) expect(hits(file(name))).toBe(0)
  })
})
