// Sync history's pure model: the last 30 days, newest first, grouped by day, with the four filters. Built on the
// same ActivityRow the flyout reads (popoverContract) and the sync_activity_summary counts (syncActivityModel).
import type { ActivityRow } from '../../../popoverContract'
import type { SyncActivitySummary } from '../../../windows/syncActivityModel'
import type { HistoryRow } from '../../source/fixtures/photos'
import type { SyncProblem } from '../../source/extras'

export type { HistoryRow }
export type Filter = 'all' | 'this-pc' | 'other' | 'problems'
export const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' }, { id: 'this-pc', label: 'This PC' }, { id: 'other', label: 'Other devices' }, { id: 'problems', label: 'Problems' },
]
export const WINDOW_DAYS = 30

const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const two = (n: number) => String(n).padStart(2, '0')
const dayNo = (d: Date) => Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 86_400_000)

export function dayLabel(iso: string, now: Date): string {
  const d = new Date(iso)
  const diff = dayNo(now) - dayNo(d)
  if (diff === 0) return 'Today'
  if (diff === 1) return 'Yesterday'
  return `${WEEKDAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`
}
export const timeLabel = (iso: string) => { const d = new Date(iso); return `${two(d.getHours())}:${two(d.getMinutes())}` }

export const isProblem = (r: HistoryRow) => r.problem !== undefined
export const problemCount = (rows: HistoryRow[]) => rows.filter(isProblem).length

export function applyFilter(rows: HistoryRow[], filter: Filter, query: string): HistoryRow[] {
  const q = query.trim().toLowerCase()
  return rows.filter((r) => {
    if (filter === 'this-pc' && r.scope !== 'this-pc') return false
    if (filter === 'other' && r.scope !== 'other') return false
    if (filter === 'problems' && !isProblem(r)) return false
    return !q || r.name.toLowerCase().includes(q) || r.what.toLowerCase().includes(q)
  })
}

/** Rows inside the 30 days, newest first. */
export function withinWindow(rows: HistoryRow[], now: Date): HistoryRow[] {
  const from = now.getTime() - WINDOW_DAYS * 86_400_000
  return rows.filter((r) => new Date(r.at).getTime() >= from).sort((a, b) => b.at.localeCompare(a.at))
}

export function groupDays(rows: HistoryRow[], now: Date): { label: string; rows: HistoryRow[] }[] {
  const out: { label: string; rows: HistoryRow[] }[] = []
  for (const r of rows) {
    const label = dayLabel(r.at, now)
    const last = out[out.length - 1]
    if (last?.label === label) last.rows.push(r)
    else out.push({ label, rows: [r] })
  }
  return out
}

/** Live: the flyout's own rows (this PC only) become history rows. A failed row says nothing was lost. */
export function rowsFromActivity(activity: ActivityRow[]): HistoryRow[] {
  return activity.filter((a) => a.state !== 'active').map((a) => {
    const failed = a.state === 'failed'
    const where = a.folder || 'Beebeeb'
    return {
      id: a.id, name: a.name, device: 'This PC', scope: 'this-pc' as const, kind: 'file' as const,
      at: new Date(a.at * 1000).toISOString(),
      what: failed ? `Didn’t sync · ${where}` : a.direction === 'down' ? `Downloaded from ${where}` : `Uploaded to ${where}`,
      problem: failed ? 'It didn’t sync. Nothing was lost; Beebeeb will try again.' : undefined,
    }
  })
}

/** Problems the queue gave up on beyond the rows listed: counts only, never a file name (sync_activity_summary). */
export const gaveUp = (s: SyncActivitySummary | null): number => s?.queue?.gave_up ?? 0

/** Live (B02b): the queue's problems, each with why in plain words. A stopped backup change names its source. */
export function rowsFromProblems(problems: SyncProblem[], now: Date): HistoryRow[] {
  return problems.map((p) => ({
    id: `problem:${p.id}`, name: p.name, device: 'This PC', scope: 'this-pc' as const, kind: 'file' as const,
    at: p.at ?? now.toISOString(), itemId: p.fileId,
    what: p.backup ? 'Didn’t back up' : `Didn’t sync · ${p.folder || 'Beebeeb'}`,
    problem: p.reason.endsWith('Nothing was lost.') ? p.reason : `${p.reason} Nothing was lost.`,
  }))
}

/** The queue's change id behind a problem row, or null for a row with nothing to retry. */
export const problemId = (r: HistoryRow): string | null => (r.id.startsWith('problem:') ? r.id.slice('problem:'.length) : null)
