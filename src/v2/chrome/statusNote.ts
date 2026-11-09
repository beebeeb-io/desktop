// A section's words for the window's status line (DW-1524: "Saved · encrypted · version 4 · 09:41"), shown in place of
// the sync words until the section clears them or new words replace them. Signed out and offline still come first.
import { useSyncExternalStore } from 'react'

let note: string | null = null
const listeners = new Set<() => void>()

export function setStatusNote(text: string | null): void {
  if (note === text) return
  note = text
  listeners.forEach((l) => l())
}

const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l) } }
export const useStatusNote = (): string | null => useSyncExternalStore(subscribe, () => note, () => null)
