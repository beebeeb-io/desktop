// The desktop v2 main window on Windows: the app-drawn caption bar, the sidebar, one section, the status line.
// Frozen after the scaffold: batches build inside sections/, chrome/ (B03) and their own directories;
// a change here goes through the lead (design/desktop-v2/BATCHES.md, "Shared files").

import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type ComponentType, type ReactElement } from 'react'
import { DriveSourceProvider, useDriveSource, type DriveSource, type StorageSummary, type SyncSummary } from '@beebeeb/shared/drive-ui'
import { CaptionBar } from './chrome/CaptionBar'
import { Sidebar } from './chrome/Sidebar'
import { StatusLine } from './chrome/StatusLine'
import { AuthGate } from './auth/Gate'
import { GuideHost } from './guide/GuideHost'
import { board } from './board'
import { useNavigation, type SectionId } from './routes'
import { useShortcuts, type CommandId } from './shortcuts'
import { pickSource } from './source'
import { useDriveExtras } from './source/extras'
import { shareList } from './sections/shared/api'
import { throttled } from './throttle'
import { onCount, onRecount, recount } from './chrome/counts'
import type { SectionProps } from './sections/types'

const SECTION_COMPONENTS: Record<SectionId, ComponentType<SectionProps>> = {
  files: lazy(() => import('./sections/files')),
  shared: lazy(() => import('./sections/shared')),
  photos: lazy(() => import('./sections/photos')),
  backups: lazy(() => import('./sections/backups')),
  trash: lazy(() => import('./sections/trash')),
  'sync-history': lazy(() => import('./sections/sync-history')),
  'available-offline': lazy(() => import('./sections/available-offline')),
  search: lazy(() => import('./sections/search')),
  settings: lazy(() => import('./sections/settings')),
  open: lazy(() => import('./sections/open')),
}

const TITLES: Record<SectionId, string> = {
  files: 'Files',
  shared: 'Shared',
  photos: 'Photos',
  backups: 'Backups',
  trash: 'Trash',
  'sync-history': 'Sync history',
  'available-offline': 'Available offline',
  search: 'Search',
  settings: 'Settings',
  open: 'Beebeeb',
}

/** The sidebar row a section lights: a file open in its viewer or editor is still in Files (M3d). */
const sidebarRow = (section: SectionId): SectionId => (section === 'open' ? 'files' : section)

/** The window with its chrome, for the app and for frames that show the whole window. */
export function V2Shell() {
  const source = useDriveSource()
  const nav = useNavigation()
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [sync, setSync] = useState<SyncSummary | null>(null)
  const counts = useSidebarCounts(nav.route.section)
  // Search is a view over the section it opened from: the sidebar keeps that one current (DW-0801 to DW-0803, M3c).
  const [searchFrom, setSearchFrom] = useState<SectionId>('files')
  if (nav.route.section !== 'search' && nav.route.section !== searchFrom) setSearchFrom(nav.route.section)

  useEffect(() => {
    source.storage().then(setStorage, () => setStorage(null))
    source.sync().then(setSync, () => setSync(null))
    return source.subscribe((e) => {
      if (e.kind === 'storage') setStorage(e.storage)
      if (e.kind === 'sync') setSync(e.sync)
    })
  }, [source])

  const onCommand = useCallback((command: CommandId) => {
    if (command === 'settings') nav.go({ section: 'settings' })
    else if (command === 'find' && nav.route.section !== 'open') nav.go({ section: 'search' })
    else if (command === 'back') nav.back()
    else if (command === 'forward') nav.forward()
  }, [nav])
  useShortcuts(onCommand)

  const Section = SECTION_COMPONENTS[nav.route.section]
  const title = nav.route.section === 'files' ? 'Beebeeb' : `${TITLES[nav.route.section]} - Beebeeb`

  return (
    <div className="flex h-screen flex-col bg-paper font-sans text-ink">
      <CaptionBar title={title} />
      <div className="flex min-h-0 flex-1">
        <Sidebar active={sidebarRow(nav.route.section === 'search' ? searchFrom : nav.route.section)} onGo={(section) => nav.go({ section })} storage={storage} accountName="Account" counts={counts} />
        <main className="flex min-w-0 flex-1 flex-col">
          <Suspense fallback={null}>
            <Section nav={nav} />
          </Suspense>
        </main>
      </div>
      <StatusLine sync={sync} />
      <GuideHost nav={nav} />
    </div>
  )
}

/**
 * The sidebar's counts (DW-0401: "Shared (1 request waiting)", "Trash (12)"; DW-1301: "Sync history 3"): invitations
 * waiting from share_list, Trash from DriveSource, sync problems from the extras. Unknown stays absent.
 * When each is read: at start and when the window gets focus; the Shared and Trash sections' own reloads publish theirs
 * (./chrome/counts.ts); `removed` re-reads Trash and `sync` the sync problems (a local read), each at most once a
 * second; moving to another section re-reads both. A `changed` event reads nothing: the engine's poller sends one every
 * tick while it syncs, and share_list and drive_trash ask the server.
 */
function useSidebarCounts(section: SectionId): Partial<Record<SectionId, number>> {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [counts, setCounts] = useState<Partial<Record<SectionId, number>>>({})
  const reread = useRef<(() => void) | null>(null)
  useEffect(() => {
    let live = true
    // Only the newest answer for a count lands: an older read that answers later never writes over it.
    const turn: Partial<Record<SectionId, number>> = {}
    const next = (id: SectionId) => (turn[id] = (turn[id] ?? 0) + 1)
    const land = (id: SectionId, n: number, count: number | undefined) => {
      if (live && turn[id] === n) setCounts((c) => ({ ...c, [id]: count }))
    }
    const read = <T,>(id: SectionId, ask: () => Promise<T>, count: (v: T) => number | undefined) => {
      const n = next(id)
      void ask().then((v) => land(id, n, count(v)), () => land(id, n, undefined))
    }
    const shares = () => read('shared', shareList, (r) => (r.ok ? r.value.invitations.length : undefined))
    const trash = () => read('trash', () => source.trash(), (t) => t.length)
    const problems = () => read('sync-history', () => extras.syncProblems(), (p) => p.length)
    const all = () => { shares(); trash(); problems() }
    all()
    reread.current = () => { trash(); problems() }
    const offCount = onCount((id, count) => land(id, next(id), count))
    const readers: Partial<Record<SectionId, () => void>> = { shared: shares, trash, 'sync-history': problems }
    const offRecount = onRecount((id) => readers[id]?.())
    const onRemoved = throttled(trash)
    const onSync = throttled(problems)
    const stop = source.subscribe((e) => {
      if (e.kind === 'removed') onRemoved()
      else if (e.kind === 'sync') onSync()
    })
    window.addEventListener('focus', all)
    return () => {
      live = false
      reread.current = null
      offCount(); offRecount(); onRemoved.cancel(); onSync.cancel(); stop()
      window.removeEventListener('focus', all)
    }
  }, [source, extras])
  const first = useRef(true)
  useEffect(() => {
    if (first.current) { first.current = false; return }
    reread.current?.()
  }, [section])
  return counts
}

/** Undo and Restore answer with the engine's `changed`, which the sidebar does not read on: they ask for the Trash count. */
function recountingTrash(source: DriveSource): DriveSource {
  return {
    ...source,
    undoTrash: (token) => source.undoTrash(token).finally(() => recount('trash')),
    restoreFromTrash: (ids) => source.restoreFromTrash(ids).finally(() => recount('trash')),
  }
}

function FrameNotBuilt({ id }: { id: string }) {
  return (
    <div className="grid h-screen place-items-center bg-paper font-sans text-[13px] text-ink-3">
      Frame {id} is not built yet.
    </div>
  )
}

export default function V2App() {
  const source = useMemo(() => recountingTrash(pickSource()), [])
  // `?frame=DW-xxxx` draws a board frame (headless screenshots, the map audit). The frame table is the dev server's
  // (registered by ./dev/live.ts through ./board.ts): a production build has none, so the parameter opens the window.
  const frames = board<Record<string, () => ReactElement>>('frames')
  const frame = frames ? new URLSearchParams(window.location.search).get('frame') : null
  const renderFrame = frame ? frames?.[frame] : undefined
  return (
    <DriveSourceProvider source={source}>
      {frame ? (renderFrame ? renderFrame() : <FrameNotBuilt id={frame} />) : (
        <AuthGate>
          <V2Shell />
        </AuthGate>
      )}
    </DriveSourceProvider>
  )
}
