// The desktop v2 main window on Windows: the app-drawn caption bar, the sidebar, one section, the status line.
// Frozen after the scaffold: batches build inside sections/, chrome/ (B03) and their own directories;
// a change here goes through the lead (design/desktop-v2/BATCHES.md, "Shared files").

import { lazy, Suspense, useCallback, useEffect, useMemo, useState, type ComponentType } from 'react'
import { DriveSourceProvider, useDriveSource, type StorageSummary, type SyncSummary } from '@beebeeb/shared/drive-ui'
import { CaptionBar } from './chrome/CaptionBar'
import { Sidebar } from './chrome/Sidebar'
import { StatusLine } from './chrome/StatusLine'
import { AuthGate } from './auth/Gate'
import { FRAMES } from './frames/registry'
import { useNavigation, type SectionId } from './routes'
import { useShortcuts, type CommandId } from './shortcuts'
import { pickSource } from './source'
import { useDriveExtras } from './source/extras'
import { shareList } from './sections/shared/api'
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

/** The window with its chrome, for the app and for frames that show the whole window. */
export function V2Shell() {
  const source = useDriveSource()
  const nav = useNavigation()
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [sync, setSync] = useState<SyncSummary | null>(null)
  const counts = useSidebarCounts()

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
    else if (command === 'find') nav.go({ section: 'search' })
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
        <Sidebar active={nav.route.section} onGo={(section) => nav.go({ section })} storage={storage} accountName="Account" counts={counts} />
        <main className="flex min-w-0 flex-1 flex-col">
          <Suspense fallback={null}>
            <Section nav={nav} />
          </Suspense>
        </main>
      </div>
      <StatusLine sync={sync} />
    </div>
  )
}

/**
 * The sidebar's counts from the sources (DW-0401: "Shared (1 request waiting)", "Trash (12)"; DW-1301: "Sync history 3"):
 * invitations waiting from share_list, Trash from DriveSource, sync problems from the extras. Unknown stays absent.
 */
function useSidebarCounts(): Partial<Record<SectionId, number>> {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [counts, setCounts] = useState<Partial<Record<SectionId, number>>>({})
  useEffect(() => {
    let live = true
    const read = () => {
      void Promise.all([
        shareList().then((r) => (r.ok ? r.value.invitations.length : undefined)),
        source.trash().then((t) => t.length, () => undefined),
        extras.syncProblems().then((p) => p.length, () => undefined),
      ]).then(([shared, trash, problems]) => { if (live) setCounts({ shared, trash, 'sync-history': problems }) })
    }
    read()
    const stop = source.subscribe((e) => { if (e.kind === 'changed' || e.kind === 'removed' || e.kind === 'sync') read() })
    return () => { live = false; stop() }
  }, [source, extras])
  return counts
}

function FrameNotBuilt({ id }: { id: string }) {
  return (
    <div className="grid h-screen place-items-center bg-paper font-sans text-[13px] text-ink-3">
      Frame {id} is not built yet.
    </div>
  )
}

export default function V2App() {
  const source = useMemo(() => pickSource(), [])
  const frame = new URLSearchParams(window.location.search).get('frame')
  const renderFrame = frame ? FRAMES[frame] : undefined
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
