/**
 * What a Review row's button does (task 1683 slice 4): one place for Review and for Home's "Needs your attention", so the
 * same press means the same thing in both. "Decide…" hands the entry to whoever opens the conflict sheet; "Add storage"
 * and "Manage plan" open the billing page in the browser; "Sign in again" starts the in-place sign-in (R8; nothing is cleared). The
 * commands it needs come in as arguments, so a test can script them.
 */
import { BILLING_URL, type CommandResult, type SessionActionOutcome, type VersionConflictEntry } from './desktopApi'
import type { ReviewRowAction } from './reviewModel'

interface ReviewActionDeps {
  showToast: (toast: { variant: 'error' | 'info'; title?: string; message: string }) => unknown
  onDecide: (entry: VersionConflictEntry) => void
  openUrl: (url: string) => Promise<CommandResult<void>>
  forceReauth: () => Promise<CommandResult<SessionActionOutcome>>
}

export function makeReviewActions({ showToast, onDecide, openUrl, forceReauth }: ReviewActionDeps) {
  const signInAgain = async () => {
    // `forceReauth` decides the flow per platform; on a Mac it opens sign-in in place and clears nothing (R8).
    const result = await forceReauth()
    if (!result.ok) {
      showToast({ variant: 'error', title: 'Couldn’t open sign-in', message: result.reason })
      return
    }
    // The expired session was cleared, but a step could not be confirmed: said neutrally (FB-24).
    if (result.value.warning) showToast({ variant: 'info', message: result.value.warning.sentence })
  }

  return (entry: VersionConflictEntry, action: ReviewRowAction): void => {
    if (action.kind === 'open_conflict') onDecide(entry)
    else if (action.kind === 'upgrade' || action.kind === 'manage_plan') void openUrl(BILLING_URL)
    else void signInAgain()
  }
}
