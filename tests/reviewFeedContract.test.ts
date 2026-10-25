/**
 * The contract between Rust's Review feed and the window (task 1683 slice 4; spec §7.6, hi-fi section 3). Rust's test
 * `the_review_feed_is_exactly_the_shared_fixture` (`engine_bridge.rs`) proves the JSON it produces IS
 * `tests/fixtures/review-feed.entries.json`; this reads the same file with the window's validator and row words. A field
 * either side adds, drops or renames fails one of the two. What it does NOT prove: the engine's choice of entries
 * (src-tauri) or the pane's layout.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { readReviewEntries } from '../src/reviewFeed'
import { parseReviewEntries, parseReviewEntry, reviewKindLabel, reviewRowAction, reviewRowMeta } from '../src/reviewModel'

const fixture = JSON.parse(readFileSync(new URL('./fixtures/review-feed.entries.json', import.meta.url), 'utf8')) as unknown[]
const now = new Date(1_790_001_000 * 1000)

describe('the entries Rust sends (6 of 6)', () => {
  test('every one passes the validator, none is dropped, and the field names are the ones the type has', () => {
    const entries = parseReviewEntries(fixture)
    expect(entries).not.toBeNull()
    expect(entries!.length).toBe(fixture.length)
    expect(entries!.map((e) => e.id)).toEqual(['conflict:f-board', 'op:op-upload', 'op:op-root', 'op:op-deep', 'op:op-parked', 'op:op-plan'])
    const keys = fixture.map((entry) => Object.keys(entry as object).sort())
    for (const row of keys) {
      expect(row).toEqual(['action', 'base_version', 'detail', 'file_id', 'file_name', 'folder', 'id', 'kind', 'last_error', 'op_id', 'status', 'updated_at', 'version_id'])
    }
  })

  test('the folder is the parent directory\'s name, and a file at the root has none', () => {
    const [board, upload, root, deep, parked, plan] = parseReviewEntries(fixture)!
    expect(board.folder).toBe('Team shared')
    expect(upload.folder).toBe('Photos')
    expect(root.folder).toBeNull()
    expect(deep.folder).toBe('2026')
    // An edit of an existing file: Rust's queue holds only the leaf, and the folder still comes from the file's own row.
    expect(parked.folder).toBe('Team shared')
    expect(plan.folder).toBe('Invoices')
  })

  test('a row says folder, then when, then the detail, and a root-level file starts with when', () => {
    const [board, upload, root] = parseReviewEntries(fixture)!
    expect(reviewRowMeta(board, new Date(2026, 9, 25, 14, 30))).toMatch(/^Team shared · \d{1,2} \w{3}( \d{4})? · Changed on two devices\.$/)
    expect(reviewRowMeta(upload, now)).toMatch(/^Photos · .* · Couldn’t upload, will retry$/)
    expect(reviewRowMeta(root, now)).toMatch(/^[^·]+ · This change can’t upload until you free up space or add storage\. It stays on this Mac\.$/)
    expect(reviewRowMeta(root, now).startsWith('null')).toBe(false)
  })

  test('the kinds Rust names are the kinds the window has words for, and the actions it names are the ones it can do', () => {
    const entries = parseReviewEntries(fixture)!
    expect(entries.map((e) => reviewKindLabel(e.kind))).toEqual(['Conflict', 'Upload', 'Storage full', 'Signed out', 'Not sent', 'Plan'])
    expect(entries.map((e) => reviewRowAction(e)?.label ?? null)).toEqual(['Decide…', null, 'Add storage', 'Sign in again', null, 'Manage plan'])
  })
})

describe('the validator refuses what is not an entry (9 of 9)', () => {
  const good = fixture[0] as Record<string, unknown>
  test('each way an entry can be wrong', () => {
    const broken: Array<[string, (e: Record<string, unknown>) => void]> = [
      ['no id', (e) => { delete e.id }],
      ['a file name that is not text', (e) => { e.file_name = 7 }],
      ['no kind', (e) => { delete e.kind }],
      ['no detail', (e) => { delete e.detail }],
      ['a folder that is a number', (e) => { e.folder = 3 }],
      ['a time that is text', (e) => { e.updated_at = 'yesterday' }],
      ['a base version that is text', (e) => { e.base_version = '4' }],
      ['a last error that is a list', (e) => { e.last_error = [] }],
      ['no action', (e) => { delete e.action }],
    ]
    let refused = 0
    for (const [label, mutate] of broken) {
      const copy = JSON.parse(JSON.stringify(good)) as Record<string, unknown>
      mutate(copy)
      if (parseReviewEntry(copy) !== null) throw new Error(`accepted an entry with: ${label}`)
      refused += 1
    }
    expect(refused).toBe(9)
    for (const raw of [null, undefined, 'x', 4, [], true]) expect(parseReviewEntry(raw)).toBeNull()
  })

  test('an entry that does not read is dropped, the rest stay; an answer that is not a list is no feed', () => {
    const feed = parseReviewEntries([fixture[0], { id: 'x' }, fixture[1], null])!
    expect(feed.map((e) => e.id)).toEqual(['conflict:f-board', 'op:op-upload'])
    expect(parseReviewEntries(null)).toBeNull()
    expect(parseReviewEntries({ entries: [] })).toBeNull()
    expect(parseReviewEntries('[]')).toBeNull()
  })
})

describe('the feed reads through the validator', () => {
  const withBackend = async <T>(answer: () => unknown, run: () => Promise<T>) => {
    const previous = (globalThis as any).window
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async (name: string) => { if (name !== 'list_version_conflict_center') throw new Error(`unscripted ${name}`); return answer() } } }
    try { return await run() } finally { (globalThis as any).window = previous }
  }

  test('rows from Rust come through whole; one that does not read is dropped', async () => {
    const result = await withBackend(() => [fixture[0], { id: 'x' }, fixture[2]], readReviewEntries)
    expect(result.ok && result.value.entries.map((e) => e.id)).toEqual(['conflict:f-board', 'op:op-root'])
    expect(result.ok && result.value.rejected).toBe(1)
  })

  test('an answer that is not a list is a failure the pane can show, not an empty Review', async () => {
    const result = await withBackend(() => ({ unexpected: true }), readReviewEntries)
    expect(result).toEqual({ ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false })
  })

  test('a command failure is passed through untouched', async () => {
    const result = await withBackend(() => { throw new Error('state.db is locked') }, readReviewEntries)
    expect(result.ok).toBe(false)
    expect(!result.ok && result.reason).toBe('state.db is locked')
  })
})
