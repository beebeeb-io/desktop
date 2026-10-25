/**
 * The Versions page's counters (PR #140 delta review, P3): "Changes Beebeeb couldn’t send yet" counts what failed on its way
 * to the server and nothing that is counted elsewhere on the page. A conflict is "Active conflicts", a change made on another
 * device is "Changed elsewhere", and a restore is neither: it is not a change that failed to leave this Mac.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { reviewCounts } from '../src/reviewCounts'

const of = (...kinds: string[]) => kinds.map((kind) => ({ kind })) as Parameters<typeof reviewCounts>[0]

describe('what could not be sent yet', () => {
  test('counts every kind of upload-side failure', () => {
    for (const kind of ['failed_upload', 'quota_failure', 'permission_failure', 'metadata', 'delete', 'auth_failure']) {
      expect(reviewCounts(of(kind)), kind).toEqual({ notSent: 1, changedElsewhere: 0 })
    }
  })

  test('does not count a conflict, a change made elsewhere, or a restore: each is counted somewhere else', () => {
    expect(reviewCounts(of('conflict')).notSent).toBe(0)
    expect(reviewCounts(of('stale_base')).notSent).toBe(0)
    expect(reviewCounts(of('restore')).notSent).toBe(0)
  })

  test('a mixed list counts each entry in at most one place', () => {
    const counts = reviewCounts(of('conflict', 'conflict', 'failed_upload', 'quota_failure', 'stale_base', 'restore', 'delete'))
    expect(counts).toEqual({ notSent: 3, changedElsewhere: 1 })
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
