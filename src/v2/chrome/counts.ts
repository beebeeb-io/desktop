//! The sidebar's counts follow the sections' own reloads: a section that has just read a list the sidebar counts
//! publishes its length here (share_list's invitations, the Trash list), so an accept in Shared shows in the sidebar at
//! once without a second read; an action whose engine event the sidebar does not read on (Undo, Restore: `changed`)
//! asks for a recount. V2App's useSidebarCounts listens.
import type { SectionId } from '../routes'

type Listener = (section: SectionId, count: number) => void
const listeners = new Set<Listener>()

export function publishCount(section: SectionId, count: number): void {
  listeners.forEach((l) => l(section, count))
}

export function onCount(listener: Listener): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

type Recount = (section: SectionId) => void
const recounts = new Set<Recount>()

export function recount(section: SectionId): void {
  recounts.forEach((l) => l(section))
}

export function onRecount(listener: Recount): () => void {
  recounts.add(listener)
  return () => { recounts.delete(listener) }
}
