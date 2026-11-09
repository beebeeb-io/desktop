// DW-0401's status line: "Up to date · 4 on this PC · 3 available offline · 4 online only".
// The words for each phase come from the board; batch B3 (global elements) owns the full set.

import { useEffect, useState } from 'react'
import type { SyncSummary } from '@beebeeb/shared/drive-ui'
import { useDriveExtras, type TransferRow } from '../source/extras'
import { fmtSize } from '../sections/files/model'
import { mergeMoving } from '../sections/files/liveExtras'
import { useStatusNote } from './statusNote'

const PHASE: Record<SyncSummary['phase'], string> = {
  checking: 'Checking…',
  'up-to-date': 'Up to date',
  syncing: 'Syncing',
  paused: 'Paused',
  // DW-1108: "The status line: Offline".
  offline: 'Offline',
  'needs-decision': 'Needs a decision',
  stopped: 'Stopped',
}

/**
 * DW-0426: while files upload the line says so, "Uploading 2 of 4 · 42 MB left" (moving of all uploads; the bytes
 * left of those not stopped), and goes back to Up to date when everything is done. Null when nothing uploads.
 */
export function uploadingLine(rows: TransferRow[]): string | null {
  const up = rows.filter((r) => r.direction === 'up')
  const live = up.filter((r) => r.state !== 'stopped')
  const moving = live.filter((r) => r.state === 'moving').length
  // Queued uploads with nothing moving are not "Uploading 0 of 3": the phase says why (Offline, Paused).
  if (moving === 0) return null
  const known = live.every((r) => (r.total ?? 0) > 0)
  const left = live.reduce((n, r) => n + Math.max(0, (r.total ?? 0) - r.bytes), 0)
  return [`Uploading ${moving} of ${up.length}`, known ? `${fmtSize(left)} left` : null].filter(Boolean).join(' · ')
}

function useUploading(sync: SyncSummary | null): string | null {
  const extras = useDriveExtras()
  const [rows, setRows] = useState<TransferRow[]>([])
  // The moving rows arrive on their own event (one subscription); the whole queue is read again with each summary.
  useEffect(() => extras.subscribeTransfers((moving) => setRows((prev) => mergeMoving(prev, moving))), [extras])
  useEffect(() => {
    let live = true
    extras.transfers().then((r) => { if (live) setRows(r) }, () => { if (live) setRows([]) })
    return () => { live = false }
  }, [extras, sync])
  return uploadingLine(rows)
}

/** B02b's drive_status, read with each sync summary: a session that ended (DW-1109), the first list (DW-1107). */
function useEngine(sync: SyncSummary | null): { signedOut: boolean; listing: boolean } {
  const extras = useDriveExtras()
  const [state, setState] = useState({ signedOut: false, listing: false })
  useEffect(() => {
    let live = true
    extras.status().then((s) => { if (live) setState({ signedOut: s.signedOut, listing: !!s.listing }) }, () => { if (live) setState({ signedOut: false, listing: false }) })
    return () => { live = false }
  }, [extras, sync])
  return state
}

/**
 * The line's words. Up to date and syncing show the counts (DW-0401); a state that needs attention says only that:
 * Signed out (DW-1109), Uploading … (DW-0426), Getting your files ready (DW-1107), Offline (DW-1108).
 */
export function statusWords(sync: SyncSummary, o: { signedOut?: boolean; uploading?: string | null; listing?: boolean } = {}): string {
  if (o.signedOut) return 'Signed out'
  if (sync.phase === 'offline') return PHASE.offline
  if (o.uploading && sync.phase !== 'paused') return o.uploading
  if (o.listing) return 'Getting your files ready'
  return [
    sync.phase === 'syncing' && sync.pending > 0 ? `Syncing ${sync.pending}` : PHASE[sync.phase],
    `${sync.onDevice} on this PC`,
    `${sync.availableOffline} available offline`,
    `${sync.onlineOnly} online only`,
  ].join(' · ')
}

export function StatusLine({ sync }: { sync: SyncSummary | null }) {
  const { signedOut, listing } = useEngine(sync)
  const uploading = useUploading(sync)
  // A section's own words (a save, DW-1524) stand in for the sync words, never for Signed out or Offline.
  const note = useStatusNote()
  if (!sync) return <footer className="h-7 flex-none border-t border-line bg-paper-2" />
  // B02b: the status line opens the Uploads panel over it (DW-0427); Files listens for the event.
  return (
    <footer role="status" className="flex h-7 flex-none items-center border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
      <button type="button" data-open-uploads onClick={() => window.dispatchEvent(new CustomEvent('bb:uploads-open'))}
        className="inline-flex items-center gap-1.5 rounded-sm text-left hover:text-ink focus-visible:outline-2 focus-visible:outline-ink-3">
        {signedOut && <span aria-hidden="true" className="h-1.5 w-1.5 flex-none rounded-full bg-red" />}
        {note && !signedOut && sync.phase !== 'offline' ? note : statusWords(sync, { signedOut, uploading, listing })}
      </button>
    </footer>
  )
}
