// B12's data layer. Everything comes through command() from desktopApi (never the shared API). A plain browser on the
// dev server (headless frames, bun run dev) has no IPC bridge: the board's values come from ../../../board.ts, which
// the dev server fills from source/fixtures/settings.ts. The running app never shows them (P1c): before a read answers,
// and when it fails, a pane shows the app's own defaults or its honest empty state.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useDriveSource, type StorageSummary as DriveStorage } from '@beebeeb/shared/drive-ui'
import { command, DEFAULT_CONFIG, type DesktopConfig, type StorageSummary } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { board } from '../../../board'

/**
 * Desktop settings: read once; each change is written as a patch through set_desktop_config. The command merges only
 * the fields it is sent (CLAUDE.md "Never break": merge-only, never the whole object read on open), and a `pause_sync`
 * in it pauses or resumes the engine, so a write sends only what the person changed.
 */
export function useConfig(initial: Partial<DesktopConfig> = {}) {
  // In the window, the app's own defaults until the read answers, never the board's example numbers (#270 follow-up,
  // P1c); `loaded` says when the read has answered.
  const [config, setConfig] = useState<DesktopConfig>({ ...(hasTauri() ? DEFAULT_CONFIG : board<DesktopConfig>('config') ?? DEFAULT_CONFIG), ...initial })
  const [loaded, setLoaded] = useState(() => !hasTauri())
  const [error, setError] = useState<string | null>(null)
  // One write at a time, in the order they were made: Pause then Resume never lands as Resume then Pause.
  const writes = useRef<Promise<unknown>>(Promise.resolve())
  // The fields changed here: a read that answers after a change never writes over it (#270 follow-up).
  const changed = useRef(new Set<string>())
  useEffect(() => {
    if (!hasTauri()) return
    let live = true
    command<DesktopConfig>('get_desktop_config').then((r) => {
      if (!live) return
      if (r.ok) {
        setConfig((c) => {
          const read: Record<string, unknown> = { ...DEFAULT_CONFIG, ...r.value, ...initial }
          for (const k of changed.current) read[k] = (c as unknown as Record<string, unknown>)[k]
          return read as unknown as DesktopConfig
        })
      } else setError(r.reason)
      setLoaded(true)
    })
    return () => { live = false }
  }, [])
  /** Resolves when this change is written (true) or refused (false), so a caller that reads the config from Rust
   *  next waits for it (#274 review: a channel check ran before the new channel was saved). */
  const update = useCallback((patch: Partial<DesktopConfig>): Promise<boolean> => {
    for (const k of Object.keys(patch)) changed.current.add(k)
    setConfig((c) => ({ ...c, ...patch }))
    // Outside the state updater, which React may run twice: one change, one write.
    if (!hasTauri()) return Promise.resolve(true)
    const written = writes.current
      .then(() => command('set_desktop_config', { config: patch }))
      .then((r) => { if (!r.ok) setError(r.reason); return r.ok })
    writes.current = written
    return written
  }, [])
  return { config, update, error, loaded }
}

export interface KnownFolder {
  key: string
  display_name: string
  enabled: boolean
  item_count: number | null
  /** The board's size and last-backup text (a frame's); the desktop reports an item count instead. */
  size?: string
  last?: string
  custom?: boolean
}

/**
 * This PC's backed-up folders (`get_known_folder_backup`, `set_known_folder_backup`). Empty until the read answers;
 * a failed read says so (`error`), never the board's folders.
 */
export function useKnownFolders() {
  const [folders, setFolders] = useState<KnownFolder[]>(() => (hasTauri() ? [] : board<KnownFolder[]>('folders') ?? []))
  const [error, setError] = useState(false)
  useEffect(() => {
    if (!hasTauri()) return
    command<KnownFolder[]>('get_known_folder_backup').then((r) => { if (r.ok) setFolders(r.value); else setError(true) })
  }, [])
  const toggle = useCallback((key: string, enabled: boolean) => {
    setFolders((fs) => fs.map((f) => (f.key === key ? { ...f, enabled } : f)))
    // Only the board's rows are custom folders (no command adds one), and the board has no Rust behind it.
    if (hasTauri()) command('set_known_folder_backup', { key, enabled })
  }, [])
  return { folders, toggle, error }
}

/** What a read gives a pane: the value once it answered, the refusal, or still waiting. */
export type Read<T> = { state: 'loading' } | { state: 'ready'; value: T } | { state: 'failed'; reason: string }

/** One read command, answered by Rust in the window and by the dev server's host in a plain browser (dev/reads.ts). */
export function useRead<T>(name: string, args?: Record<string, unknown>): Read<T> & { reload: () => void } {
  const [read, setRead] = useState<Read<T>>({ state: 'loading' })
  const key = JSON.stringify(args ?? null)
  const load = useCallback(() => {
    let live = true
    void command<T>(name, args).then((r) => { if (live) setRead(r.ok ? { state: 'ready', value: r.value } : { state: 'failed', reason: r.reason }) })
    return () => { live = false }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, key])
  useEffect(() => load(), [load])
  return { ...read, reload: () => { load() } }
}

/** This PC's copies (`desktop_storage_summary`): downloaded files (`cache_bytes`) and Available offline (`pinned_bytes`). */
export function useLocalSpace(): Read<StorageSummary> {
  return useRead<StorageSummary>('desktop_storage_summary')
}

/** The account's storage, split by area (DriveSource's `storage()`, `drive_storage`). */
export function useAccountStorage(): Read<DriveStorage> {
  const source = useDriveSource()
  const [read, setRead] = useState<Read<DriveStorage>>({ state: 'loading' })
  useEffect(() => {
    let live = true
    source.storage().then((value) => { if (live) setRead({ state: 'ready', value }) }, (e: unknown) => { if (live) setRead({ state: 'failed', reason: e instanceof Error ? e.message : String(e) }) })
    return () => { live = false }
  }, [source])
  return read
}
