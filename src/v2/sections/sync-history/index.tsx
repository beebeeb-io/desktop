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
import { gaveUp, problemId, rowsFromActivity, rowsFromProblems, withinWindow, type Filter, type HistoryRow } from './model'
import { useDriveExtras } from '../../source/extras'

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [rows, setRows] = useState<HistoryRow[]>([])
  const [filter, setFilter] = useState<Filter>('all')
  const [query, setQuery] = useState('')
  const [queued, setQueued] = useState(0)
  const now = useMemo(() => new Date(), [])

  const load = useCallback(async () => {
    const [snap, summary, problems] = await Promise.all([popoverSnapshot(200), loadSyncActivitySummary(), extras.syncProblems().catch(() => [])])
    const at = new Date()
    // The queue's rows say why; the flyout's failed rows are the same failures without a reason, so they give way.
    const activity = snap.ok ? rowsFromActivity(snap.value.activity).filter((r) => !(problems.length > 0 && r.problem)) : []
    setRows(withinWindow([...rowsFromProblems(problems, at), ...activity], at))
    setQueued(gaveUp(summary))
  }, [extras])
  useEffect(() => {
    load().catch(() => {})
    return source.subscribe((e) => { if (e.kind === 'sync') load().catch(() => {}) })
  }, [source, load])

  const retry = async (only?: HistoryRow) => {
    const ids = (only ? [only] : rows.filter((r) => r.problem)).map(problemId).filter((id): id is string => id !== null)
    try {
      const { retried } = await extras.retry(ids)
      showToast(retried > 0 ? { kind: 'done', text: 'Trying again' } : { kind: 'failed', text: 'Beebeeb is already trying again' })
    } catch {
      showToast({ kind: 'failed', text: 'Couldn’t try again' })
    }
    await load()
  }
  return (
    <>
      <HistoryView
        rows={rows} now={now} filter={filter} query={query} onFilter={setFilter} onQuery={setQuery} onTry={(id) => { void retry(rows.find((r) => r.id === id)) }} onTryAll={() => { void retry() }}
        onOpen={(r) => { if (r.trashed) nav.go({ section: 'trash' }); else if (r.itemId) nav.go({ section: 'files', id: null }) }}
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
