/**
 * The two counters on the Versions page that are not "Active conflicts": what could not be sent yet, and what changed
 * elsewhere (PR #140 delta review, P3). Every entry is counted in exactly one place on that page.
 */
import type { VersionConflictEntry } from './desktopApi'

/**
 * What failed on its way to the server. Not here, because each is counted elsewhere on the page: a conflict ("Active
 * conflicts"), a change made on another device (`stale_base`, "Changed elsewhere") and a restore, which is not a change
 * that failed to leave this Mac.
 */
const COULD_NOT_BE_SENT: ReadonlyArray<VersionConflictEntry['kind']> = [
  'failed_upload',
  'quota_failure',
  'permission_failure',
  'metadata',
  'delete',
  'auth_failure',
]

export function reviewCounts(entries: ReadonlyArray<Pick<VersionConflictEntry, 'kind'>>): { notSent: number; changedElsewhere: number } {
  return {
    notSent: entries.filter((entry) => COULD_NOT_BE_SENT.includes(entry.kind)).length,
    // Task 1640: a kept copy (changes kept from a file deleted elsewhere) is counted here too.
    changedElsewhere: entries.filter((entry) => entry.kind === 'stale_base' || entry.kind === 'kept_copy').length,
  }
}
