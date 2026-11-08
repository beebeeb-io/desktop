// Sync history, live: the flyout's rows (popover_snapshot) for this PC, and the queue's counts (sync_activity_summary).
// Other devices' changes and a per-row failure reason are not in either command: they stay absent (the PR's gap list).
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { popoverSnapshot } from '../../../desktopApi'
import { loadSyncActivitySummary } from '../../../desktopApi'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { HistoryView } from './HistoryView'
import { gaveUp, rowsFromActivity, withinWindow, type Filter, type HistoryRow } from './model'

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const [rows, setRows] = useState<HistoryRow[]>([])
  const [filter, setFilter] = useState<Filter>('all')
  const [query, setQuery] = useState('')
  const [queued, setQueued] = useState(0)
  const now = useMemo(() => new Date(), [])

  const load = useCallback(async () => {
    const [snap, summary] = await Promise.all([popoverSnapshot(200), loadSyncActivitySummary()])
    if (snap.ok) setRows(withinWindow(rowsFromActivity(snap.value.activity), new Date()))
    setQueued(gaveUp(summary))
  }, [])
  useEffect(() => {
    load().catch(() => {})
    return source.subscribe((e) => { if (e.kind === 'sync') load().catch(() => {}) })
  }, [source, load])

  const retry = async () => {
    // Retrying is the engine's own: resume sync, then read the queue again.
    showToast({ kind: 'done', text: 'Trying again' })
    await load()
  }
  return (
    <>
      <HistoryView
        rows={rows} now={now} filter={filter} query={query} onFilter={setFilter} onQuery={setQuery} onTry={retry} onTryAll={retry}
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
