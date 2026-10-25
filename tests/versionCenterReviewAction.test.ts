/**
 * Task 1546:
 *   - finding 1 — a `quota_failure` review row had NO actionable button at
 *     all (VersionCenter.tsx only wired one for `entry.action ===
 *     'open_conflict'`), unlike Windows' StorageWidget "Upgrade" button.
 *   - finding 3 — a `failed_upload` row caused by an expired session (401)
 *     rendered under the generic "Upload" label with no "sign in again" path.
 *
 * `reviewEntryAction` is the single decision VersionCenter renders its one
 * review-row button through, replacing the old inline
 * `entry.action === 'open_conflict'` check.
 */
import { describe, expect, test } from 'bun:test'
import { reviewEntryAction, reviewMetrics, reviewNeedsAttention } from '../src/desktopApi'

describe('reviewEntryAction', () => {
  test('a kept copy (task 1640) gets a Dismiss button: it needs no decision', () => {
    expect(reviewEntryAction({ kind: 'kept_copy', action: 'dismiss' })).toEqual({
      kind: 'dismiss',
      label: 'Dismiss',
    })
  })

  test('a kept copy whose original still waits to be removed gets no button yet', () => {
    expect(reviewEntryAction({ kind: 'kept_copy', action: 'none' })).toBeNull()
  })

  test('a real conflict still gets the Review button', () => {
    expect(reviewEntryAction({ kind: 'conflict', action: 'open_conflict' })).toEqual({
      kind: 'open_conflict',
      label: 'Review',
    })
  })

  test('a quota-blocked row gets an Upgrade button (finding 1)', () => {
    expect(reviewEntryAction({ kind: 'quota_failure', action: 'review_upload' })).toEqual({
      kind: 'upgrade',
      label: 'Upgrade',
    })
  })

  test('an auth-failure row gets a Sign in again button (finding 3)', () => {
    expect(reviewEntryAction({ kind: 'auth_failure', action: 'review_upload' })).toEqual({
      kind: 'sign_in_again',
      label: 'Sign in again',
    })
  })

  test('a plain failed upload / permission / stale-base row still gets no button', () => {
    expect(reviewEntryAction({ kind: 'failed_upload', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'permission_failure', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'stale_base', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'restore', action: 'restore_review' })).toBeNull()
  })
})

// 0.8.13 hand port (task 1640): the review page's count and its dot.
describe('the review count and dot (0.8.13 hand port of task 1640)', () => {
  test('a kept copy counts as changed elsewhere, never as a failed send', () => {
    // 0.8.12's own rule stays: a stale entry counts in both.
    expect(reviewMetrics([{ kind: 'kept_copy' }, { kind: 'stale_base' }, { kind: 'conflict' }])).toEqual({ failed: 1, stale: 2 })
  })

  test('only kept copies leave the dot quiet; anything that needs a decision turns it red', () => {
    expect(reviewNeedsAttention(0, [{ kind: 'kept_copy' }])).toBe(false)
    expect(reviewNeedsAttention(0, [{ kind: 'kept_copy' }, { kind: 'stale_base' }])).toBe(true)
    expect(reviewNeedsAttention(1, [])).toBe(true)
  })
})
