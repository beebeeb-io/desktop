/**
 * Task 2190 (plan 2026-10-25-2190-macos-parked-conflict §3.4): a parked stale-base Finder write is
 * decided in the conflict window by its queued write (`op_id`), not by the file. The copy here is
 * what the window says on that path only.
 */
import type { ParkedResolution } from './desktopApi'

/**
 * D1 (lead, 25 Oct 2026): the second press of "Keep Theirs" on a parked conflict. It is true only
 * because the engine copies the Mac's bytes into the Changes not sent folder BEFORE it drops the
 * write, and fails with nothing changed when it cannot. So it is shown only on the path that calls
 * `resolveParkedConflict(opId, 'remote')`.
 */
export const PARKED_REPLACE_QUESTION =
  'Replace this Mac’s version? A copy of your changes stays in the Changes not sent folder.'

/** The decision found the write moved since the review was read: nothing changed. */
export const PARKED_MOVED = 'This file changed again. Close this and decide again.'

/** What the window does after a decision: close (done) with a confirmation, or stay and say why. */
export function parkedResolutionNotice(result: ParkedResolution): { done: boolean; message: string } {
  switch (result.outcome) {
    case 'rebased':
      return { done: true, message: 'This Mac’s version is being sent.' }
    case 'kept_other':
      return { done: true, message: 'The other version is used here. A copy of your changes is in Changes not sent.' }
    case 'kept_both':
      return { done: true, message: `Both are kept. This Mac’s version is now “${result.name}”.` }
    case 'moved':
      return { done: false, message: PARKED_MOVED }
  }
}
