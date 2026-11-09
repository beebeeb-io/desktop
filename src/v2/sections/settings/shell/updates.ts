// The newest version this PC knows of (M3e): what the updater last noted (popover_snapshot's `update`, the same note
// the flyout reads), its `update-available` event while Settings is open, and Check now's answer. Null: none known.
import { useEffect, useSyncExternalStore } from 'react'
import { popoverSnapshot } from '../../../../desktopApi'
import { hasTauri } from '../../../source'

let known: string | null = null
const subs = new Set<() => void>()
export function noteUpdate(version: string | null): void {
  known = version
  subs.forEach((f) => f())
}

export function useUpdateOffer(): string | null {
  const version = useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => known)
  useEffect(() => {
    let live = true
    // The snapshot raises the note, never clears it: Check now's answer this session stands (Rust notes the same version,
    // so the two agree; an offer ends when the update installs, which restarts the app).
    void popoverSnapshot(0).then((r) => { const v = r.ok ? r.value.update?.version : null; if (live && v) noteUpdate(v) })
    let off: (() => void) | undefined
    if (hasTauri()) {
      void import('@tauri-apps/api/event').then(({ listen }) => listen<{ version?: string }>('update-available', (e) => {
        if (typeof e.payload?.version === 'string') noteUpdate(e.payload.version)
      })).then((stop) => { if (live) off = stop; else stop() })
    }
    return () => { live = false; off?.() }
  }, [])
  return version
}
