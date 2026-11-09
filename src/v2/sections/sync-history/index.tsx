// Sync history, live: the flyout's rows (popover_snapshot) for this PC, and the queue's counts (sync_activity_summary).
// Problems come from B02b's drive_sync_problems with a per-row reason, and Try again is drive_sync_retry. Other devices'
// changes are in no command on this PC: they stay absent.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { popoverSnapshot } from '../../../desktopApi'
import { loadSyncActivitySummary } from '../../../desktopApi'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { HistoryView } from './HistoryView'
import { gaveUp, mergeHistory, problemId, routeFilter, type Filter, type HistoryRow } from './model'
import { useDriveExtras } from '../../source/extras'

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [rows, setRows] = useState<HistoryRow[]>([])
  // The sync problem banner's View lands on Problems (DW-0102 → DW-0428): `sub=problems`, also while already here.
  // The route sets it both ways: arriving with `sub=problems` shows Problems, and a route without it (Back, the sidebar)
  // shows All again, so the filter never sticks on Problems after leaving (#268).
  const [filter, setFilter] = useState<Filter>(routeFilter(nav.route.sub))
  useEffect(() => { setFilter(routeFilter(nav.route.sub)) }, [nav.route])
  const [query, setQuery] = useState('')
  const [queued, setQueued] = useState(0)
  // A failed read says so; the rows it already had stay (never a blank history).
  const [failed, setFailed] = useState(false)
  const now = useMemo(() => new Date(), [])

  const load = useCallback(async () => {
    try {
      const [snap, summary, problems] = await Promise.all([popoverSnapshot(200), loadSyncActivitySummary(), extras.syncProblems()])
      if (!snap.ok) throw new Error(snap.reason)
      // The queue's rows say why; the flyout's copies of those same failures give way.
      setRows(mergeHistory(snap.value.activity, problems, new Date()))
      setQueued(gaveUp(summary))
      setFailed(false)
    } catch {
      setFailed(true)
    }
  }, [extras])
  useEffect(() => {
    load().catch(() => {})
    return source.subscribe((e) => { if (e.kind === 'sync') load().catch(() => {}) })
  }, [source, load])

  const retry = async (only?: HistoryRow) => {
    // Only a row the queue knows a change id for can be retried; the toast says how many it actually took.
    const ids = (only ? [only] : rows.filter((r) => r.problem)).map(problemId).filter((id): id is string => id !== null)
    if (ids.length === 0) return
    try {
      const { retried } = await extras.retry(ids)
      showToast(retried > 0
        ? { kind: 'done', text: `Trying again · ${retried} ${retried === 1 ? 'item' : 'items'}` }
        : { kind: 'failed', text: 'Beebeeb is already trying again' })
    } catch {
      showToast({ kind: 'failed', text: 'Couldn’t try again' })
    }
    await load()
  }
  /** Double-click (DW-1301): the file in the window, in its folder; a trashed one opens Trash; a problem, which never
   *  reached Beebeeb, is shown in File Explorer when Beebeeb knows it (DW-0428). */
  const open = async (r: HistoryRow) => {
    if (r.trashed) return nav.go({ section: 'trash' })
    if (!r.itemId) return showToast({ kind: 'failed', text: 'This one isn’t in Beebeeb: it didn’t sync.' })
    if (r.problem) return source.revealInFileManager(r.itemId).catch(() => showToast({ kind: 'failed', text: 'Couldn’t show it in File Explorer' }))
    try {
      const { item } = await source.details(r.itemId)
      nav.go({ section: 'files', id: item.isFolder ? item.id : item.parentId })
    } catch {
      showToast({ kind: 'failed', text: 'Couldn’t find it in Beebeeb. It may have moved or gone to Trash.' })
    }
  }
  return (
    <>
      {failed && (
        <div role="alert" data-error="sync-history" className="flex flex-none items-center gap-2 border-b border-line bg-paper-2 px-4 py-2 text-[12px] text-ink-2">
          {rows.length > 0
            ? 'Beebeeb couldn’t load your sync history. Showing what it last knew.'
            : 'Beebeeb couldn’t load your sync history.'}
        </div>
      )}
      <HistoryView
        rows={rows} now={now} filter={filter} query={query} onFilter={setFilter} onQuery={setQuery} onTry={(id) => { void retry(rows.find((r) => r.id === id)) }} onTryAll={() => { void retry() }}
        onOpen={(r) => { void open(r) }}
        onMenu={(id, r) => {
          if (id === 'copy') void navigator.clipboard?.writeText(r.name)
          else if (id === 'reveal' && r.itemId) source.revealInFileManager(r.itemId).catch(() => showToast({ kind: 'failed', text: 'Couldn’t show it in File Explorer' }))
          else if (id === 'beebeeb') nav.go({ section: r.trashed ? 'trash' : 'files' })
        }}
      />
      {queued > 0 && <span hidden data-gave-up={queued} />}
    </>
  )
}
