// Drive › Sync and File Explorer's "Turn off" confirm (DW-0746b). The pane itself and its repair alert are today's
// (../today/sync.tsx); B12's first versions of both, drawn with the board's numbers, are gone (P1c).
import { useEffect, useState } from 'react'
import { command } from '../../../../desktopApi'
import { Btn, Dialog } from '../shell/kit'
import { useSettings } from '../shell/context'
import { actionFailure } from '../../files/model'

/** The changes made on this PC that have not uploaded yet, in the confirm's words. */
export function unsentLine(total: number | null): string {
  if (total === null) return 'Changes made in File Explorer that have not uploaded yet would be lost.'
  if (total === 0) return 'Every change made on this PC has uploaded.'
  return `${total.toLocaleString('en')} ${total === 1 ? 'change' : 'changes'} made on this PC ${total === 1 ? 'has' : 'have'} not uploaded yet and would be lost.`
}

/**
 * DW-0746b. Blocked (M3e): no command takes Beebeeb out of File Explorer; Turn off says so and changes nothing. The
 * count is the engine's (`unsent_changes`, its `total`), never the board's "2 files, 14 MB" (P1c).
 */
export function ExplorerOffDialog() {
  const { close } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const [total, setTotal] = useState<number | null>(null)
  useEffect(() => {
    let live = true
    void command<{ total: number }>('unsent_changes', { offset: 0, limit: 1 }).then((r) => { if (live && r.ok && typeof r.value.total === 'number') setTotal(r.value.total) })
    return () => { live = false }
  }, [])
  return (
    <Dialog labelId="exp-h" title="Turn off Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" onClick={() => setRefused(`${actionFailure('turn off Beebeeb in File Explorer', 'unsupported').text} Nothing was changed.`)}>Turn off</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      <p>{unsentLine(total)}</p>
      <p className="mt-2">Files that have uploaded are safe. The window keeps working.</p>
    </Dialog>
  )
}
