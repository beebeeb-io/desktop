/**
 * Review's pure decisions (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6, §10.2
 * rows 11 to 17 and 35): the kind tags, the one action per row, when a row happened, the sidebar badge, the conflict
 * sheet's choices and the restore toast. Nothing here renders; tests/reviewPane.test.tsx runs the pane.
 */
import { describe, expect, test } from 'bun:test'
import { REVIEW_ENTRY_KINDS, reviewEntryAction, type VersionConflictEntry } from '../src/desktopApi'
import {
  CONFLICT_CHOICES,
  CONFLICT_REPLACE_CANCEL,
  CONFLICT_REPLACE_GUARD_MS,
  CONFLICT_REPLACE_LABEL,
  CONFLICT_REPLACE_QUESTION,
  CONFLICT_RESOLVED_TOAST,
  CONFLICT_SHEET_INTRO,
  CONFLICT_SIDE_LABELS,
  REVIEW_EMPTY,
  REVIEW_INTRO,
  REVIEW_SOME_UNREADABLE,
  VERSIONS_LOAD_FAILED,
  conflictSheetTitle,
  parseReviewEntries,
  parseReviewFeed,
  restoreQueuedToast,
  reviewActionName,
  reviewBadge,
  reviewKindLabel,
  reviewMoreLine,
  reviewRowAction,
  reviewRowMeta,
  reviewWhen,
  versionLabel,
} from '../src/reviewModel'

const now = new Date(2026, 9, 25, 14, 30) // Sunday 25 October 2026, 14:30 local
const at = (y: number, mo: number, d: number, h = 0, mi = 0) => new Date(y, mo, d, h, mi).getTime() / 1000

const entry = (over: Partial<VersionConflictEntry> = {}): VersionConflictEntry => ({
  id: 'op:1',
  file_id: 'f1',
  file_name: 'Site visit, north wing.mov',
  kind: 'failed_upload',
  status: 'upload review',
  updated_at: at(2026, 9, 25, 8, 47),
  detail: 'Couldn’t upload, will retry',
  action: 'review_upload',
  ...over,
})

describe('the kind tags (§10.2 row 16)', () => {
  test('the four renamed tags, and the ones that stay', () => {
    expect(reviewKindLabel('stale_base')).toBe('Changed elsewhere')
    expect(reviewKindLabel('metadata')).toBe('Rename or move')
    expect(reviewKindLabel('quota_failure')).toBe('Storage full')
    expect(reviewKindLabel('permission_failure')).toBe('No permission')
    expect(reviewKindLabel('conflict')).toBe('Conflict')
    expect(reviewKindLabel('failed_upload')).toBe('Upload')
    expect(reviewKindLabel('delete')).toBe('Delete')
    expect(reviewKindLabel('restore')).toBe('Restore')
  })

  test('a sign-in failure is not called a conflict (the old fall-through did), and a kind nobody named says Other', () => {
    expect(reviewKindLabel('auth_failure')).toBe('Signed out')
    expect(reviewKindLabel('from_a_newer_engine')).toBe('Other')
  })

  test('a change that was not sent, and a plan that refuses uploads, have words of their own (review of #142)', () => {
    expect(reviewKindLabel('not_sent')).toBe('Not sent')
    expect(reviewKindLabel('account_refused')).toBe('Plan')
    expect(reviewKindLabel('other')).toBe('Other')
  })

  test('every kind Rust can send has a tag of its own (only "other" is called Other)', () => {
    for (const kind of REVIEW_ENTRY_KINDS) {
      if (kind !== 'other') expect(reviewKindLabel(kind), kind).not.toBe('Other')
    }
    expect(reviewKindLabel('other')).toBe('Other')
  })

  test('none of the retired words survives', () => {
    const all = ['conflict', 'failed_upload', 'quota_failure', 'permission_failure', 'stale_base', 'restore', 'metadata', 'delete', 'auth_failure', 'not_sent', 'account_refused', 'other'].map(reviewKindLabel)
    for (const retired of ['Stale base', 'Metadata', 'Quota', 'Permission']) expect(all).not.toContain(retired)
  })
})

describe('one action per row (§10.2 row 17)', () => {
  test('a conflict says "Decide…", a storage failure "Add storage", a sign-in failure "Sign in again"', () => {
    expect(reviewRowAction(entry({ kind: 'conflict', action: 'open_conflict' }))).toEqual({ kind: 'open_conflict', label: 'Decide…' })
    expect(reviewRowAction(entry({ kind: 'quota_failure' }))).toEqual({ kind: 'upgrade', label: 'Add storage' })
    expect(reviewRowAction(entry({ kind: 'auth_failure', action: 'sign_in_again' }))).toEqual({ kind: 'sign_in_again', label: 'Sign in again' })
  })

  test('a plan refusal says "Manage plan": adding storage would not help a lapsed trial', () => {
    expect(reviewRowAction(entry({ kind: 'account_refused' }))).toEqual({ kind: 'manage_plan', label: 'Manage plan' })
    expect(reviewEntryAction(entry({ kind: 'account_refused' }))).toBeNull() // Other platforms keep their existing contract.
  })

  test('every other kind has no action: a fake button would be worse than none', () => {
    for (const kind of ['failed_upload', 'permission_failure', 'stale_base', 'metadata', 'delete', 'restore', 'not_sent', 'other'] as const) {
      expect(reviewRowAction(entry({ kind }))).toBeNull()
    }
  })

  test('it decides exactly as the shared rule does; only the words differ (Windows and Linux keep theirs)', () => {
    const kinds = ['conflict', 'failed_upload', 'quota_failure', 'permission_failure', 'stale_base', 'restore', 'metadata', 'delete', 'auth_failure', 'kept_copy', 'not_sent', 'other'] as const
    for (const kind of kinds) {
      for (const action of ['open_conflict', 'review_upload', 'restore_review', 'sign_in_again']) {
        const e = entry({ kind, action })
        expect(reviewRowAction(e)?.kind ?? null).toBe(reviewEntryAction(e)?.kind ?? null)
      }
    }
    expect(reviewEntryAction(entry({ kind: 'conflict', action: 'open_conflict' }))?.label).toBe('Review')
    expect(reviewEntryAction(entry({ kind: 'quota_failure' }))?.label).toBe('Upgrade')
  })
})

describe('when a row happened', () => {
  test('today and yesterday say so with the time; older rows say the day', () => {
    expect(reviewWhen(at(2026, 9, 25, 9, 12), now)).toBe('today 09:12')
    expect(reviewWhen(at(2026, 9, 24, 18, 3), now)).toBe('yesterday 18:03')
    expect(reviewWhen(at(2026, 9, 12, 7, 5), now)).toBe('12 Oct')
    expect(reviewWhen(at(2025, 11, 31, 23, 59), now)).toBe('31 Dec 2025')
  })

  test('the same day of another month or year is not today, and yesterday works across a month and a year', () => {
    expect(reviewWhen(at(2026, 8, 25, 10, 0), now)).toBe('25 Sep')
    expect(reviewWhen(at(2025, 9, 25, 10, 0), now)).toBe('25 Oct 2025')
    expect(reviewWhen(at(2026, 9, 31, 22, 0), new Date(2026, 10, 1, 10, 0))).toBe('yesterday 22:00')
    expect(reviewWhen(at(2025, 11, 31, 22, 0), new Date(2026, 0, 1, 10, 0))).toBe('yesterday 22:00')
  })

  test('midnight is a day boundary, not 24 hours', () => {
    expect(reviewWhen(at(2026, 9, 25, 0, 0), now)).toBe('today 00:00')
    expect(reviewWhen(at(2026, 9, 24, 23, 59), now)).toBe('yesterday 23:59')
  })

  test('an unknown time is no text at all (no placeholder)', () => {
    expect(reviewWhen(undefined, now)).toBeNull()
    expect(reviewWhen(null, now)).toBeNull()
    expect(reviewWhen(0, now)).toBeNull()
    expect(reviewWhen(Number.NaN, now)).toBeNull()
  })

  test('milliseconds are read as milliseconds', () => {
    expect(reviewWhen(at(2026, 9, 25, 9, 12) * 1000, now)).toBe('today 09:12')
  })
})

describe('the row\'s second line (§7.6)', () => {
  test('the folder, when it happened, then Rust\'s detail line verbatim, for a conflict as for any other row', () => {
    const c = entry({ kind: 'conflict', action: 'open_conflict', folder: 'Team shared', detail: 'Changed on two devices.', updated_at: at(2026, 9, 25, 9, 12) })
    expect(reviewRowMeta(c, now)).toBe('Team shared · today 09:12 · Changed on two devices.')
    expect(reviewRowMeta(entry({ folder: 'Photos' }), now)).toBe('Photos · today 08:47 · Couldn’t upload, will retry')
  })

  test('a file at the root of the vault has no folder: the line starts with when, never with "null" or a separator', () => {
    for (const folder of [null, undefined, '', '  ']) {
      expect(reviewRowMeta(entry({ folder }), now)).toBe('today 08:47 · Couldn’t upload, will retry')
    }
  })

  test('without a time the line is the detail alone, and without either it is empty', () => {
    expect(reviewRowMeta(entry({ updated_at: undefined }), now)).toBe('Couldn’t upload, will retry')
    expect(reviewRowMeta(entry({ updated_at: undefined, detail: '' }), now)).toBe('')
    expect(reviewRowMeta(entry({ detail: '  ' }), now)).toBe('today 08:47')
    expect(reviewRowMeta(entry({ folder: 'Photos', updated_at: undefined, detail: '' }), now)).toBe('Photos')
  })
})

describe('what a screen reader calls a row\'s button (WCAG 2.5.3: the visible word comes first)', () => {
  test('"Decide…" names its file; the account-wide buttons keep their word', () => {
    const e = entry({ file_name: 'Board notes, 29 Sept.md', kind: 'conflict', action: 'open_conflict' })
    expect(reviewActionName(e, reviewRowAction(e)!)).toBe('Decide…, Board notes, 29 Sept.md')
    for (const kind of ['quota_failure', 'account_refused', 'auth_failure'] as const) {
      const row = entry({ kind })
      const action = reviewRowAction(row)!
      expect(reviewActionName(row, action)).toBe(action.label)
    }
  })
})

describe('the entries Rust sends, read one by one (review of #142)', () => {
  const good = { id: 'op:1', file_id: 'f1', file_name: 'a.md', kind: 'failed_upload', status: 's', detail: 'd', action: 'review_upload' }

  test('the rows that read are kept and the rows that did not are counted, never dropped without a word', () => {
    const feed = parseReviewFeed([good, { id: 'x' }, null, good, 'text'])!
    expect(feed.entries).toHaveLength(2)
    expect(feed.rejected).toBe(3)
    expect(parseReviewFeed([good])).toEqual({ entries: [good], rejected: 0 })
    expect(parseReviewFeed([])).toEqual({ entries: [], rejected: 0 })
  })

  test('not a list is no feed, for both readers of it', () => {
    for (const raw of [null, undefined, {}, 'x', 4]) {
      expect(parseReviewFeed(raw)).toBeNull()
      expect(parseReviewEntries(raw)).toBeNull()
    }
    expect(parseReviewEntries([good, { id: 'x' }])).toEqual([good])
  })

  test('the words under the rows', () => {
    expect(REVIEW_SOME_UNREADABLE).toBe('Some items couldn’t be shown.')
    expect(reviewMoreLine(9, 2)).toBe('7 more are waiting. They appear here as you clear these.')
    expect(reviewMoreLine(3, 2)).toBe('1 more is waiting. It appears here once you clear one of these.')
    expect(reviewMoreLine(2, 2)).toBeNull()
    expect(reviewMoreLine(0, 0)).toBeNull()
    expect(reviewMoreLine(1, 4)).toBeNull() // a count that lags the list says nothing
  })
})

describe('the sidebar badge (§7.6)', () => {
  test('a count while the vault is unlocked and something waits; nothing at zero; nothing while locked', () => {
    expect(reviewBadge(2, true)).toBe(2)
    expect(reviewBadge(0, true)).toBeNull()
    expect(reviewBadge(2, false)).toBeNull()
  })
})

describe('the conflict sheet (§7.6, §10.2 row 35)', () => {
  test('title, intro and the two sides, in the words of the hi-fi', () => {
    expect(conflictSheetTitle('Board notes, 29 Sept.md')).toBe('Conflict: Board notes, 29 Sept.md')
    expect(CONFLICT_SHEET_INTRO).toBe('This file was modified on two devices. Choose which version to keep.')
    expect(CONFLICT_SIDE_LABELS).toEqual({ local: 'This Mac', remote: 'Other device' })
  })

  test('three choices in this order, mapped to the engine\'s three, and only "Keep both" is the amber primary', () => {
    expect(CONFLICT_CHOICES).toEqual([
      { choice: 'local', label: 'Keep this Mac’s', primary: false },
      { choice: 'remote', label: 'Keep the other', primary: false },
      { choice: 'both', label: 'Keep both', primary: true },
    ])
  })

  test('"Keep the other" asks first, in a question that claims only what it asks (the device rung of task 2190 sets the final copy)', () => {
    expect(CONFLICT_REPLACE_QUESTION).toBe('Use the other version here?')
    expect(CONFLICT_REPLACE_LABEL).toBe('Use the other version')
    expect(CONFLICT_REPLACE_CANCEL).toBe('Cancel')
    expect(CONFLICT_REPLACE_GUARD_MS).toBe(600)
    for (const claim of ['discard', 'Replace', 'overwrite', 'Finder', 'this Mac’s version']) {
      expect(`${CONFLICT_REPLACE_QUESTION} ${CONFLICT_REPLACE_LABEL}`, claim).not.toContain(claim)
    }
  })

  test('the success toast has no check mark (the old window printed one)', () => {
    expect(CONFLICT_RESOLVED_TOAST).toEqual({ variant: 'success', message: 'Conflict resolved' })
    expect(JSON.stringify(CONFLICT_RESOLVED_TOAST)).not.toContain('✓')
  })
})

describe('version history copy (§10.2 rows 14 and 15)', () => {
  test('the restore toast', () => {
    expect(restoreQueuedToast('Q3 ledger.xlsx', 3)).toEqual({
      variant: 'success',
      title: 'Restore queued',
      message: 'Q3 ledger.xlsx goes back to version 3 on the next sync.',
    })
  })

  test('a version without a number says "an earlier version", never an id', () => {
    expect(restoreQueuedToast('Q3 ledger.xlsx', null).message).toBe('Q3 ledger.xlsx goes back to an earlier version on the next sync.')
  })

  test('the retired engine wording is gone', () => {
    expect(JSON.stringify(restoreQueuedToast('a', 1))).not.toContain('sync engine')
    expect(VERSIONS_LOAD_FAILED).toBe('Couldn’t load this file’s versions. Try again.')
  })

  test('a version row is "Version n", "Version n · current" when it is, and "Earlier version" without a number', () => {
    expect(versionLabel({ id: 'v', version_number: 4, is_current: false })).toBe('Version 4')
    expect(versionLabel({ id: 'v', version_number: 5, is_current: true })).toBe('Version 5 · current')
    expect(versionLabel({ id: 'a-uuid-nobody-should-read', version_number: null, is_current: false })).toBe('Earlier version')
  })
})

describe('Review\'s own words (§7.6, §10.2 rows 11 and 13)', () => {
  test('intro and empty state', () => {
    expect(REVIEW_INTRO).toBe('Files that need a decision from you, and changes Beebeeb couldn’t send yet.')
    expect(REVIEW_EMPTY).toBe('Nothing needs your attention.')
  })
})
