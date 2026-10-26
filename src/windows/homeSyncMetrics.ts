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
}

/**
 * Every queued change not sent yet: waiting (due, backing off or being sent), held (paused, any reason) and given up.
 * "…" before the first read, "—" when the summary could not be read: an unknown queue is never shown as 0. Note the
 * queue also holds the downloads a pin queues, so during a pin this figure counts those too.
 */
export function notSentFigure(summary: SyncActivitySummary | null | undefined): string {
  if (summary === undefined) return '…'
  if (summary === null) return '—'
  const queue = summary.queue
  if (queue === null) return formatCount(0)
  const held = queue.held.reduce((sum, group) => sum + group.count, 0)
  return formatCount(queue.waiting + held + queue.gave_up)
}

/** State, Not sent and Conflicts, in the strip's order. "In vault" is the caller's (it comes from billing usage). */
export function homeSyncMetrics(
  status: SyncStatus | null,
  summary: SyncActivitySummary | null | undefined,
  options: SyncActivityOptions = {},
): HomeSyncMetric[] {
  return [
    { k: 'State', v: syncActivityView(status, summary, options).label },
    { k: 'Not sent', v: notSentFigure(summary) },
    { k: 'Conflicts', v: status ? String(status.conflicts) : '…' },
  ]
}
