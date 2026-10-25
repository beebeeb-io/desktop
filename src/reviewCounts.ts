/**
 * The two counters on the Versions page that are not "Active conflicts": what could not be sent yet, and what changed
 * elsewhere (PR #140 delta review, P3). An entry is counted in at most one place on that page, and a restore is counted in
 * none: restores are listed, not counted, because a restore is not a change that failed to leave this Mac.
 */
import type { VersionConflictEntry } from './desktopApi'

/**
 * What failed on its way to the server. Not here: a conflict (counted as "Active conflicts"), a change made on another device
 * (`stale_base`, counted as "Changed elsewhere") and a restore (counted nowhere, only listed).
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
    changedElsewhere: entries.filter((entry) => entry.kind === 'stale_base').length,
  }
}
