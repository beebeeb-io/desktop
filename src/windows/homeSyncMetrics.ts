/**
 * The sync figures on the Windows main window's Home strip (0.8.15): State, Not sent, Conflicts.
 *
 * On Guus's PC (a local 0.8.15 build) Home said "Up to date · Syncing 0" while 2,924 changes were on hold, 43 had given
 * up and the startup check was still running: it read `sync_status` alone. State is now the label of the one model that
 * Settings › Sync and the tray render (`syncActivityView`), so Home can never say "Up to date" where they do not, and the
 * second figure counts the changes the queue has not sent instead of the rows mid-transfer.
 *
 * No words are decided here: State is the model's label, and "Not sent" is the product's phrase for queued changes
 * ("Sign out with changes not sent?", `signOutCopy.ts`).
 */
import type { SyncStatus } from '../desktopApi'
import { formatCount, syncActivityView, type SyncActivityOptions, type SyncActivitySummary } from './syncActivityModel'

export interface HomeSyncMetric {
  k: string
  v: string
  /** Task 2409: the figure opens Activity › Not sent, where the changes it counts are listed (only when it counts any). */
  notSentLink?: boolean
}

/**
 * Every change not sent yet: waiting (due, backing off or being sent), held (paused, any reason) and given up. Null
 * before the first read, when the summary could not be read, or when it carries no queue (no state DB yet). Since task
 * 2409 the summary leaves out the downloads a pin queues (task 2224): they are not changes leaving this PC.
 */
export function notSentCount(summary: SyncActivitySummary | null | undefined): number | null {
  const queue = summary?.queue ?? null
  if (queue === null) return null
  return queue.waiting + queue.held.reduce((sum, group) => sum + group.count, 0) + queue.gave_up
}

/**
 * The "Not sent" figure: {@link notSentCount}, "…" before the first read, "—" when the summary could not be read or
 * carries no queue: an unknown queue is never shown as 0.
 */
export function notSentFigure(summary: SyncActivitySummary | null | undefined): string {
  if (summary === undefined) return '…'
  const count = notSentCount(summary)
  return count === null ? '—' : formatCount(count)
}

/** State, Not sent and Conflicts, in the strip's order. "In vault" is the caller's (it comes from billing usage). */
export function homeSyncMetrics(
  status: SyncStatus | null,
  summary: SyncActivitySummary | null | undefined,
  options: SyncActivityOptions = {},
): HomeSyncMetric[] {
  return [
    { k: 'State', v: syncActivityView(status, summary, options).label },
    { k: 'Not sent', v: notSentFigure(summary), notSentLink: (notSentCount(summary) ?? 0) > 0 },
    { k: 'Conflicts', v: status ? String(status.conflicts) : '…' },
  ]
}
