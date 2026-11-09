// The open file's version, watched while it is open (DW-1519, DW-1524), for the text editor and viewers and for Office.
import { useCallback, useEffect, useRef, useState } from 'react'
import type { DriveSource, VersionEntry } from '@beebeeb/shared/drive-ui'
import { tagContent, tagVersion, versionNews, type SaveNote, type VersionWatch } from './model'

/** What the watch reads to compare a landed version with this window's save: Rust's tag of the file's content now. */
export type ReadTag = (id: string) => Promise<string>

/**
 * Desktop's engine says something changed with a `changed` or `sync` event (drive_v2 events.rs); each one re-reads the
 * item's version number from the state DB (drive_details' versionCount, a local read). A newer version is announced,
 * never swapped in under the reader. This window's own save is "Saved on this PC · encrypting…" until the engine has
 * uploaded it: the version moved past the one it was saved over, and the file holds the save's content (Rust's tag).
 * Then it is "Saved · encrypted · version N", N from Rust. A restore lands as a restore (#273 findings 3, 8, 12).
 * `id` null watches nothing (a new document before its first save).
 */
export function useVersionWatch(source: DriveSource, id: string | null, base: number | null, on: {
  readTag: ReadTag
  /** This window's restore landed as `version`. */
  restored?: (version: number) => void
  /** This window's save is on the server as `version`; `tag` is Rust's tag of it. */
  saved?: (version: number, tag: string) => void
}) {
  const [newer, setNewer] = useState<VersionEntry | null>(null)
  const [note, setNote] = useState<SaveNote | null>(null)
  const watch = useRef<VersionWatch>({ base: 0, pending: null })
  const callbacks = useRef(on)
  callbacks.current = on
  useEffect(() => { if (base !== null && !watch.current.pending) watch.current.base = Math.max(watch.current.base, base) }, [base])
  // One re-read at a time; an event during one asks for one more after it.
  const run = useRef<'idle' | 'busy' | 'again'>('idle')
  const check = useCallback(async () => {
    if (!id) return
    if (run.current !== 'idle') { run.current = 'again'; return }
    const again = () => run.current === 'again'
    try {
      do {
        run.current = 'busy'
        const { item, versionCount } = await source.details(id)
        const p = watch.current.pending
        // While a save waits, Rust's tag of the file is read: its number is the server's (0 for a file never uploaded,
        // which drive_details shows as 1) and its content part tells this save from another device's version.
        const tag = p?.kind === 'save' && item.local !== 'syncing' ? await callbacks.current.readTag(id) : null
        const version = tag === null ? versionCount : tagVersion(tag) ?? versionCount
        const news = versionNews(watch.current, { version, local: item.local, content: tag === null ? undefined : tagContent(tag) })
        if (news === 'same') continue
        // Another device's version while this window's save waits: the save still waits, now over that version.
        watch.current = { base: Math.max(version, versionCount), pending: news === 'news' && p?.kind === 'save' ? { ...p, from: version } : null }
        if (news === 'saved') { setNote({ kind: 'encrypted', version }); if (tag !== null) callbacks.current.saved?.(version, tag) }
        else if (news === 'restored') callbacks.current.restored?.(versionCount)
        else {
          const v = await source.versions(id).catch((): VersionEntry[] => [])
          setNewer(v.find((x) => x.current) ?? { id: `v${versionCount}`, createdAt: new Date().toISOString(), size: item.size ?? 0, current: true })
        }
      } while (again())
    } catch { /* the next event reads again */ } finally { run.current = 'idle' }
  }, [source, id])
  // A watch that starts with a save waiting (a new document's first save, before its id was known) reads at once.
  useEffect(() => { if (watch.current.pending) void check() }, [check])
  useEffect(() => source.subscribe((e) => {
    if (e.kind === 'changed' || e.kind === 'sync' || (e.kind === 'item' && e.item.id === id)) void check()
  }), [source, id, check])
  return {
    newer,
    note,
    dismiss: () => setNewer(null),
    clearNote: () => setNote(null),
    /** This window saved (`tag` is Rust's answer to the save; a new file made by Rust has none, and is version 0 until
     *  its first upload): saved on this PC until the engine has it. */
    savedHere: (tag: string | null) => {
      const from = tag === null ? 0 : tagVersion(tag) ?? watch.current.base
      watch.current = { ...watch.current, pending: { kind: 'save', from, content: tag === null ? null : tagContent(tag) } }
      setNewer(null)
      setNote({ kind: 'local' })
      void check()
    },
    /** This window restored an older version: it reloads when the restore lands, announced as a restore. */
    restoredHere: () => {
      watch.current = { ...watch.current, pending: { kind: 'restore', from: watch.current.base } }
      setNewer(null)
      setNote(null)
      void check()
    },
  }
}
