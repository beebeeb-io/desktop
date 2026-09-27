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
import { reviewEntryAction } from '../src/desktopApi'

describe('reviewEntryAction', () => {
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
