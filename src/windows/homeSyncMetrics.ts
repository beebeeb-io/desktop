/**
 * The sync figures on the Windows main window's Home strip (0.8.15): State, Not sent, Conflicts; and, while a pin's files
 * did not download, Not downloaded (task 2414).
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

/** The Activity sections a figure opens: where what it counts is listed (`NavSection`, `WindowsApp/nav.tsx`). */
export type HomeMetricLink = 'not-sent' | 'not-downloaded'

export interface HomeSyncMetric {
  k: string
  v: string
  /**
   * Task 2409: "Not sent" opens Activity › Not sent, where the changes it counts are listed; task 2414: "Not downloaded"
   * opens Activity › Not downloaded. Only when the figure counts any.
   */
  link?: HomeMetricLink
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
 * Task 2414: a pin's downloads that are paused or out of attempts (never part of {@link notSentCount}). Null as
 * {@link notSentCount} is.
 */
export function notDownloadedCount(summary: SyncActivitySummary | null | undefined): number | null {
  return summary?.queue?.not_downloaded ?? null
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

/**
 * State, Not sent and Conflicts, in the strip's order; Not downloaded after Not sent, only while it counts any (task
 * 2414: shown as Not sent is, a link to its list). "In vault" is the caller's (it comes from billing usage).
 */
export function homeSyncMetrics(
  status: SyncStatus | null,
  summary: SyncActivitySummary | null | undefined,
  options: SyncActivityOptions = {},
): HomeSyncMetric[] {
  const notDownloaded = notDownloadedCount(summary) ?? 0
  return [
    { k: 'State', v: syncActivityView(status, summary, options).label },
    { k: 'Not sent', v: notSentFigure(summary), ...((notSentCount(summary) ?? 0) > 0 ? { link: 'not-sent' as const } : {}) },
    ...(notDownloaded > 0 ? [{ k: 'Not downloaded', v: formatCount(notDownloaded), link: 'not-downloaded' as const }] : []),
    { k: 'Conflicts', v: status ? String(status.conflicts) : '…' },
  ]
}
