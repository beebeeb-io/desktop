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

  test('a row the plan refuses gets Manage plan, not Upgrade: adding storage does not cure a lapsed trial (review of #142)', () => {
    expect(reviewEntryAction({ kind: 'account_refused', action: 'review_upload' })).toEqual({
      kind: 'manage_plan',
      label: 'Manage plan',
    })
  })

  test('a plain failed upload / permission / stale-base row still gets no button', () => {
    expect(reviewEntryAction({ kind: 'failed_upload', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'permission_failure', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'stale_base', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'restore', action: 'restore_review' })).toBeNull()
    expect(reviewEntryAction({ kind: 'not_sent', action: 'review_upload' })).toBeNull()
    expect(reviewEntryAction({ kind: 'other', action: 'review_upload' })).toBeNull()
  })
})

describe('the Linux Versions page draws every kind of action the rule can return', () => {
  // The page is JSX over a dozen hooks, so this reads its source: an action kind added to `reviewEntryAction` without a
  // branch there would fall through to "Sign in again" (it did once, for the plan refusal).
  test('open_conflict, upgrade and manage_plan each have a branch; sign_in_again is what remains', async () => {
    const source = await Bun.file(new URL('../src/pages/VersionCenter.tsx', import.meta.url)).text()
    expect(source).toContain("reviewAction.kind === 'open_conflict'")
    expect(source).toMatch(/reviewAction\.kind === 'upgrade' \|\| reviewAction\.kind === 'manage_plan'/)
    expect(source).toContain('signInAgain()')
  })

  test('its kind tags name the kinds a Mac names, and none of them is "Conflict" by fall-through', async () => {
    const source = await Bun.file(new URL('../src/pages/VersionCenter.tsx', import.meta.url)).text()
    for (const [kind, tag] of [['auth_failure', 'Signed out'], ['account_refused', 'Plan'], ['not_sent', 'Not sent'], ['other', 'Other']]) {
      expect(source).toContain(`case '${kind}':\n      return '${tag}'`)
    }
  })
})
