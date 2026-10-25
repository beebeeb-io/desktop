/**
 * The Versions page's counters (PR #140 delta review, P3; #142 review, round 2): "Changes Beebeeb couldn’t send yet" counts
 * what failed on its way to the server and nothing that is counted elsewhere on the page. A conflict is "Active conflicts", a
 * change made on another device is "Changed elsewhere", and a restore is counted nowhere (listed, not counted). EVERY KIND Rust can send is counted in
 * exactly one place: #140's list was written before #142 added `not_sent`, `account_refused` and `other`, and a given-up
 * upload was counted nowhere on Linux.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { REVIEW_ENTRY_KINDS } from '../src/desktopApi'
import { COUNTED_IN, reviewCounts } from '../src/reviewCounts'

const of = (...kinds: string[]) => kinds.map((kind) => ({ kind })) as Parameters<typeof reviewCounts>[0]
const sent = JSON.parse(readFileSync(new URL('./fixtures/review-feed.kinds.json', import.meta.url), 'utf8')) as string[]

describe('every kind has one place', () => {
  test('the kinds this window knows are the kinds Rust sends (src-tauri pins the same file): add one on either side and a test fails', () => {
    expect([...REVIEW_ENTRY_KINDS].sort()).toEqual([...sent].sort())
  })

  test('each kind is counted in exactly one place, and the table has no kind that is not sent', () => {
    expect(Object.keys(COUNTED_IN).sort()).toEqual([...REVIEW_ENTRY_KINDS].sort())
    for (const kind of REVIEW_ENTRY_KINDS) {
      const counts = reviewCounts(of(kind))
      const places = (counts.notSent > 0 ? 1 : 0) + (counts.changedElsewhere > 0 ? 1 : 0) + (COUNTED_IN[kind] === 'conflicts' || COUNTED_IN[kind] === 'restores' ? 1 : 0)
      expect(places, `${kind} is counted in ${places} places (${COUNTED_IN[kind]})`).toBe(1)
    }
  })

  test('a kind a newer engine sends is counted with the changes that did not go, never nowhere', () => {
    expect(reviewCounts(of('from_a_newer_engine'))).toEqual({ notSent: 1, changedElsewhere: 0 })
  })
})

describe('what could not be sent yet', () => {
  test('counts every kind of failure on the way to the server, the ones #142 added included', () => {
    for (const kind of ['failed_upload', 'quota_failure', 'permission_failure', 'metadata', 'delete', 'auth_failure', 'not_sent', 'account_refused', 'other']) {
      expect(reviewCounts(of(kind)), kind).toEqual({ notSent: 1, changedElsewhere: 0 })
    }
  })

  test('does not count a conflict, a change made elsewhere, or a restore: each is counted somewhere else', () => {
    expect(reviewCounts(of('conflict')).notSent).toBe(0)
    expect(reviewCounts(of('stale_base')).notSent).toBe(0)
    expect(reviewCounts(of('restore')).notSent).toBe(0)
  })

  test('a restore is counted nowhere: it is listed on the page, not counted', () => {
    expect(reviewCounts(of('restore'))).toEqual({ notSent: 0, changedElsewhere: 0 })
    expect(reviewCounts(of('restore', 'restore', 'restore'))).toEqual({ notSent: 0, changedElsewhere: 0 })
  })

  test('a mixed list counts each entry in at most one place', () => {
    const counts = reviewCounts(of('conflict', 'conflict', 'failed_upload', 'quota_failure', 'stale_base', 'restore', 'delete', 'not_sent'))
    expect(counts).toEqual({ notSent: 4, changedElsewhere: 1 })
  })

  test('an empty page counts nothing', () => {
    expect(reviewCounts([])).toEqual({ notSent: 0, changedElsewhere: 0 })
  })
})

describe('the page uses it', () => {
  test('VersionCenter counts through reviewCounts and no longer filters kinds itself', () => {
    const page = readFileSync(new URL('../src/pages/VersionCenter.tsx', import.meta.url), 'utf8').replace(/\r\n/g, '\n')
    expect(page).toContain('reviewCounts(entries)')
    expect(page).not.toContain("entry.kind !== 'conflict'")
    expect(page).toContain('metrics.notSent')
    expect(page).toContain('metrics.changedElsewhere')
  })
})
