// Drive › Sync and File Explorer's "Turn off" confirm (DW-0746b). The pane itself and its repair alert are today's
// (../today/sync.tsx); B12's first versions of both, drawn with the board's numbers, are gone (P1c).
import { useEffect, useState } from 'react'
import { command } from '../../../../desktopApi'
import { Btn, Dialog } from '../shell/kit'
import { useSettings } from '../shell/context'
import { actionFailure } from '../../files/model'

export type SyncRun = 'running' | 'paused' | 'stopped' | 'refused'

const READ_FAILED = 'Beebeeb couldn\u2019t read how many changes have not uploaded yet.'
const ZERO = 'Every change made on this PC has uploaded.'

/** One `unsent_changes` answer. A refused read hides the number. Paused wins over a running engine. */
export function unsentView(value: unknown, ok = true): { state: SyncRun; total: number | null } {
  if (!ok || value === null || typeof value !== 'object') return { state: 'refused', total: null }
  const raw = value as { total?: unknown; engine_state?: unknown; paused?: unknown }
  const total = typeof raw.total === 'number' && Number.isFinite(raw.total) ? raw.total : null
  if (raw.paused === true || raw.engine_state === 'paused') return { state: 'paused', total }
  if (typeof raw.engine_state !== 'string') return { state: 'stopped', total }
  if (raw.engine_state === 'running' || raw.engine_state === 'idle' || raw.engine_state === 'syncing') return { state: 'running', total }
  if (raw.engine_state === 'error' || raw.engine_state === 'refused') return { state: 'refused', total: null }
  return { state: 'stopped', total }
}

export function unsentLine(state: SyncRun, total: number | null): string {
  if (state === 'refused' || total === null) return READ_FAILED
  const body = total === 0
    ? ZERO
    : `${total.toLocaleString('en')} ${total === 1 ? 'change' : 'changes'} made on this PC ${total === 1 ? 'has' : 'have'} not uploaded yet and would be lost.`
  if (state === 'paused') return `Paused. ${body}`
  if (state === 'stopped') return `Stopped. ${body}`
  return body
}

export function ExplorerOffDialog() {
  const { close } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const [view, setView] = useState<{ state: SyncRun; total: number | null } | null>(null)
  useEffect(() => {
    let live = true
    void command<unknown>('unsent_changes', { offset: 0, limit: 1 }).then((r) => {
      if (live) setView(unsentView(r.ok ? r.value : null, r.ok))
    })
    return () => { live = false }
  }, [])
  const line = view ? unsentLine(view.state, view.total) : null
  const linked = !!view && view.state !== 'refused' && view.total !== null && view.total > 0
  return (
    <Dialog labelId="exp-h" title="Turn off Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" onClick={() => setRefused(`${actionFailure('turn off Beebeeb in File Explorer', 'unsupported').text} Nothing was changed.`)}>Turn off</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      {line ? <p>{line}</p> : null}
      {linked ? <p><a href="?section=sync-history">Sync history</a></p> : null}
      <p className="mt-2">Files that have uploaded are safe. The window keeps working.</p>
    </Dialog>
  )
}
