// DW-0401's status line: "Up to date · 4 on this PC · 3 available offline · 4 online only".
// The words for each phase come from the board; batch B3 (global elements) owns the full set.

import type { SyncSummary } from '@beebeeb/shared/drive-ui'

const PHASE: Record<SyncSummary['phase'], string> = {
  checking: 'Checking…',
  'up-to-date': 'Up to date',
  syncing: 'Syncing',
  paused: 'Paused',
  offline: 'No connection',
  'needs-decision': 'Needs a decision',
  stopped: 'Stopped',
}

export function StatusLine({ sync }: { sync: SyncSummary | null }) {
  if (!sync) return <footer className="h-7 flex-none border-t border-line bg-paper-2" />
  const parts = [
    sync.phase === 'syncing' && sync.pending > 0 ? `Syncing ${sync.pending}` : PHASE[sync.phase],
    `${sync.onDevice} on this PC`,
    `${sync.availableOffline} available offline`,
    `${sync.onlineOnly} online only`,
  ]
  // B02b: the status line opens the Uploads panel over it (DW-0427); Files listens for the event.
  return (
    <footer role="status" className="flex h-7 flex-none items-center border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
      <button type="button" data-open-uploads onClick={() => window.dispatchEvent(new CustomEvent('bb:uploads-open'))}
        className="rounded-sm text-left hover:text-ink focus-visible:outline-2 focus-visible:outline-ink-3">
        {parts.join(' · ')}
      </button>
    </footer>
  )
}
