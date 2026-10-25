/**
 * The counters on the Versions page that are not "Active conflicts": what could not be sent yet, and what changed elsewhere
 * (PR #140 delta review, P3). An entry is counted in at most one place on that page, and a restore is counted in none:
 * restores are listed, not counted, because a restore is not a change that failed to leave this Mac. EVERY KIND has a place
 * here: `COUNTED_IN` is a `Record` over the kinds Rust can send (`REVIEW_ENTRY_KINDS`), so a kind added without one does not
 * compile, and `tests/reviewCounts.test.ts` walks the list. (#142's review: a kind added after #140 was counted nowhere.)
 */
import type { ReviewEntryKind, VersionConflictEntry } from './desktopApi'

/** Where an entry is counted: "Active conflicts", "Upload reviews" (changes that could not be sent yet), "Changed elsewhere", or nowhere (`restores`: listed, not counted). */
export type CountedIn = 'conflicts' | 'notSent' | 'changedElsewhere' | 'restores'

export const COUNTED_IN: Record<ReviewEntryKind, CountedIn> = {
  conflict: 'conflicts',
  failed_upload: 'notSent',
  quota_failure: 'notSent',
  permission_failure: 'notSent',
  metadata: 'notSent',
  delete: 'notSent',
  auth_failure: 'notSent',
  not_sent: 'notSent',
  account_refused: 'notSent',
  // A download or a pin that has not finished, or a change of a kind the page has no other word for: it is something Beebeeb
  // has not finished and the person may want to look at.
  other: 'notSent',
  stale_base: 'changedElsewhere',
  restore: 'restores',
}

/** A kind this build has never heard of (a newer engine) is counted with the changes that did not go: never nowhere. */
const countedIn = (kind: string): CountedIn => (COUNTED_IN as Record<string, CountedIn | undefined>)[kind] ?? 'notSent'

export function reviewCounts(entries: ReadonlyArray<Pick<VersionConflictEntry, 'kind'>>): { notSent: number; changedElsewhere: number } {
  return {
    notSent: entries.filter((entry) => countedIn(entry.kind) === 'notSent').length,
    changedElsewhere: entries.filter((entry) => countedIn(entry.kind) === 'changedElsewhere').length,
  }
}
