// Batch B04's frames (design/desktop-v2/BATCHES.md). Owned by B04 only. Each renders the real Files components
// against the B04 fixtures (src/v2/source/fixtures/files.ts), so a frame is the product's own state, not a drawing.
import { useLayoutEffect, useState, type ReactElement, type ReactNode } from 'react'
import { DriveSourceProvider, type DriveItem, type StorageSummary, type SyncSummary } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Sidebar } from '../chrome/Sidebar'
import { StatusLine } from '../chrome/StatusLine'
import { Banner, bannerCopy } from '../chrome/Banners'
import { EmptyFiles, GettingReadyStrip } from '../chrome/EmptyStates'
import { ToastView, type ToastMessage } from '../chrome/Toast'
import { STORAGE_240, SYNC_OK } from './FrameWindow'
import type { SectionId } from '../routes'
import { FilesPane, type PaneProps } from '../sections/files/FilesPane'
import { ConflictDialog, DetailsDialog, MoveDialog, NewFolderDialog, ProtectedAlert, ShareDialog, VersionsDialog } from '../sections/files/Dialogs'
import { DragBadge, NoPreview, Page, PreviewPanel, SystemPicker, UploadsPanel } from '../sections/files/Panels'
import { SearchView } from '../sections/files/SearchView'
import { backgroundMenu, fileMenu, newMenu, sortMenu } from '../sections/files/menus'
import { Menu, type MenuEntry } from '../sections/files/ui'
import { arrange, DEFAULT_PREFS, takenMessage, type FolderPrefs } from '../sections/files/model'
import { BACKUP_META, createFilesSource, detailsOf, FILES_TREE, listingOf, VIDEO_LENGTH } from '../source/fixtures/files'

const SRC = createFilesSource()
const SRC_OFFLINE = createFilesSource({ scenario: 'offline' })
const noop = () => {}
const GB = 1024 ** 3
const STORAGE: StorageSummary = { ...STORAGE_240, usedBytes: 240 * GB, filesBytes: 24 * GB, photosBytes: 216 * GB, backupsBytes: 0 }
const NOW = '2026-11-08T12:00:00'
const COUNTS: Partial<Record<SectionId, number>> = { shared: 1, trash: 12 }

const contract = (items = FILES_TREE) => items.find((i) => i.id === 'i-contract')!

interface PaneOpts {
  folder?: string | null
  sel?: string[]
  prefs?: Partial<FolderPrefs>
  items?: DriveItem[]
  trim?: string
  detailsFor?: string
  rest?: Partial<PaneProps>
  /** Keep the listing's own order (the board's Backups order is not by name). */
  raw?: boolean
}

/** The props of one Files pane state, computed synchronously from the fixtures. */
function pane(o: PaneOpts = {}): PaneProps {
  const prefs: FolderPrefs = { ...DEFAULT_PREFS, ...o.prefs, sort: { ...DEFAULT_PREFS.sort, ...o.prefs?.sort } }
  const base = listingOf(o.folder ?? null, { items: o.items, offline: o.rest?.offline })
  let path = base.path
  if (o.trim) path = path.slice(Math.max(0, path.findIndex((c) => c.id === o.trim)))
  const listing = { ...base, path, folder: path[path.length - 1] }
  const items = o.raw ? listing.items : arrange(listing.items, prefs.sort)
  const sel = new Set(o.sel ?? [])
  const dId = o.detailsFor ?? (o.sel?.length === 1 ? o.sel[0] : undefined)
  const details = dId ? detailsOf(dId, o.items ?? FILES_TREE) : null
  return { listing, items, prefs, selection: sel, currentId: o.sel?.length === 1 ? o.sel[0] : null, details, canNew: true, backupMeta: BACKUP_META, now: new Date('2026-11-08T12:00:00'), ...o.rest }
}

/** A frame: the window with its chrome around one Files state, plus overlays. */
function Win({ p, section = 'files', top, overlay, toast, status, sync = SYNC_OK, offline, aside, counts = COUNTS, body }: {
  p?: PaneProps; section?: SectionId; top?: ReactNode; overlay?: ReactNode; toast?: ToastMessage; status?: ReactNode; sync?: SyncSummary | null; offline?: boolean; aside?: ReactNode; counts?: Partial<Record<SectionId, number>>; body?: ReactNode
}): ReactElement {
  return (
    <DriveSourceProvider source={offline ? SRC_OFFLINE : SRC}>
      <div data-frame-window className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
        <CaptionBarView title="Beebeeb" />
        <div className="flex min-h-0 flex-1">
          <Sidebar active={section} onGo={noop} storage={STORAGE} accountName="Anna" counts={counts} />
          <main className="flex min-w-0 flex-1 flex-col">
            {body ?? (p && <FilesPane p={{ ...p, top: p.top ?? top }} />)}
          </main>
          {aside}
        </div>
        {status ?? <StatusLine sync={sync} />}
        {toast && <div className="pointer-events-none absolute inset-x-0 bottom-10 z-40 flex justify-center"><ToastView message={toast} /></div>}
        {overlay}
      </div>
    </DriveSourceProvider>
  )
}

/** A menu under a toolbar control, measured after layout so the frame holds at any window width. */
function AnchoredMenu({ anchor, entries, label, align = 'left' }: { anchor: string; entries: MenuEntry[]; label: string; align?: 'left' | 'right' }) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null)
  useLayoutEffect(() => {
    const r = document.querySelector(`[data-anchor="${anchor}"]`)?.getBoundingClientRect()
    if (r) setPos({ x: align === 'right' ? r.right - 264 : r.left, y: r.bottom + 4 })
  }, [anchor, align])
  return pos ? <Menu x={pos.x} y={pos.y} entries={entries} label={label} onPick={noop} onClose={noop} /> : null
}

function RowMenu({ rowId, entries, label }: { rowId: string; entries: MenuEntry[]; label: string }) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null)
  useLayoutEffect(() => {
    const r = document.querySelector(`[data-row="${rowId}"]`)?.getBoundingClientRect()
    if (r) setPos({ x: r.left + 360, y: r.top + 20 })
  }, [rowId])
  return pos ? <Menu x={pos.x} y={pos.y} entries={entries} label={label} onPick={noop} onClose={noop} /> : null
}

const offlineBanner = <Banner kind="offline" copy={{ text: 'You’re offline.', detail: 'This list is from today at 09:12. Changes you make will upload when you’re back.' }} />
const withoutContract = FILES_TREE.filter((i) => i.id !== 'i-contract')
const copyItem: DriveItem = { ...contract(), id: 'i-contract-copy', name: 'Contract draft copy.pdf', modifiedAt: NOW, shared: { kind: 'none' }, local: 'online-only' }
const withCopy = [...FILES_TREE, copyItem]
const statusText = (text: string, dot?: 'red') => (
  <footer role="status" className="flex h-7 flex-none items-center gap-2 border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
    {dot && <span aria-hidden="true" className="h-2 w-2 rounded-full bg-red" />}{text}
  </footer>
)
const serviceBanner = (
  <div role="status" data-banner="managed" className="flex min-h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 py-2 text-[13px] leading-5 text-ink-2">
    <span className="min-w-0 flex-1"><span className="font-medium text-ink">Mail Backup manages this folder.</span> You can open and download its files.</span>
    <button type="button" className="h-7 flex-none rounded-md border border-line bg-paper px-3 text-[13px] font-medium text-ink">Settings</button>
  </div>
)

const PANEL: Partial<FolderPrefs> = { panel: true }
const GRID: Partial<FolderPrefs> = { view: 'grid' }
const BACKUPS = { section: 'backups' as SectionId, trim: 'f-backups' }

const contractPage = [
  'This agreement is made between Initlabs B.V. and the client named in the order form.',
  '1. Services. Initlabs provides end-to-end encrypted storage as described in the order form.',
  '2. Term. This agreement starts on the date of signing and runs for twelve months.',
  '3. Fees. The client pays the fees in the order form, monthly in advance.',
]

export const FRAMES: FrameRegistry = {
  // Files: the full Drive in the window
  'DW-0401': () => <Win p={pane({ sel: ['i-contract'], prefs: PANEL })} />,
  'DW-0402': () => <Win p={pane({ sel: ['i-contract'] })} overlay={<RowMenu rowId="i-contract" label="File" entries={fileMenu([contract()])} />} />,
  'DW-0403': () => <Win p={pane({ sel: ['i-budget', 'i-contract'], prefs: PANEL })} />,
  'DW-0404': () => <Win p={pane()} overlay={<AnchoredMenu anchor="sort" label="Sort" entries={sortMenu(DEFAULT_PREFS)} />} />,
  'DW-0405': () => <Win p={pane({ items: withoutContract })} counts={{ shared: 1, trash: 13 }} toast={{ kind: 'undo', text: 'Moved to Trash' }} />,
  'DW-0406': () => <Win p={pane({ items: withCopy, sel: ['i-contract-copy'] })} toast={{ kind: 'done', text: 'Duplicated' }} />,
  'DW-0407': () => <Win p={pane({ sel: ['i-contract'] })} overlay={<ShareDialog name="Contract draft.pdf" size={1_200_000} onCopy={noop} />} />,
  'DW-0408': () => (
    <Win p={pane({ sel: ['i-agreement'] })} overlay={
      <PreviewPanel name="Service agreement.pdf" meta="Page 1 of 6"><Page title="Service agreement" lines={contractPage} /></PreviewPanel>
    } />
  ),
  'DW-0408b': () => <Win p={pane({ sel: ['i-moodboard'] })} overlay={<NoPreview name="Moodboard.sketch" size={18_400_000} />} />,
  'DW-0409': () => (
    <Win p={pane({ sel: ['i-contract'] })} overlay={
      <MoveDialog
        path={[{ id: null, name: 'Files' }]} selectedId="f-documents" targetName="Documents"
        folders={listingOf(null).items.filter((i) => i.isFolder && i.id !== 'f-backups').map((i) => ({ id: i.id, name: i.name, count: i.childCount ?? 0 }))}
      />
    } />
  ),
  'DW-0410': () => <Win p={pane({ sel: ['i-contract'], rest: { renaming: { id: 'i-contract', value: 'Contract draft.pdf' } } })} />,
  'DW-0410b': () => (
    <Win p={pane({ items: withCopy, sel: ['i-contract-copy'], rest: { renaming: { id: 'i-contract-copy', value: 'Contract draft.pdf', error: takenMessage('Contract draft.pdf', false) } } })} />
  ),
  'DW-0411': () => <Win p={pane({ sel: ['i-contract'] })} overlay={<DetailsDialog item={contract()} details={detailsOf('i-contract')} added="2026-10-28T12:00:00" now={new Date(NOW)} />} />,
  'DW-0412': () => {
    const items = FILES_TREE.map((i) => (i.id === 'i-contract' ? { ...i, local: 'available-offline' as const } : i))
    return (
      <Win
        p={pane({ items, sel: ['i-contract'], prefs: PANEL, rest: { transfers: { 'i-contract': { kind: 'downloading', done: 700_000, total: 1_200_000 } } } })}
        toast={{ kind: 'done', text: 'Keeping on this PC' }}
      />
    )
  },
  'DW-0413': () => (
    <Win p={pane({ sel: ['i-contract'] })} overlay={
      <VersionsDialog
        now={new Date(NOW)}
        name="Contract draft.pdf"
        versions={[
          { id: 'v3', createdAt: '2026-11-07T16:42:00', size: 1_200_000, current: true, deviceName: 'This PC' },
          { id: 'v2', createdAt: '2026-11-07T11:05:00', size: 1_100_000, current: false, deviceName: 'Anna’s iPhone' },
          { id: 'v1', createdAt: '2026-10-28T14:20:00', size: 980_000, current: false, deviceName: 'Anna’s iPad' },
        ]}
      />
    } />
  ),
  'DW-0414': () => <Win p={pane()} overlay={<AnchoredMenu anchor="new" align="right" label="New" entries={newMenu()} />} />,
  'DW-0415': () => <Win p={pane()} overlay={<SystemPicker kind="files" title="Upload to Files" button="Upload" />} />,
  'DW-0416': () => <Win p={pane()} overlay={<SystemPicker kind="photos" title="Upload to Files" button="Upload 3" />} />,
  'DW-0417': () => <Win p={pane()} overlay={<NewFolderDialog parentName="Files" value="Untitled folder" />} />,
  'DW-0480': () => <Win p={pane()} overlay={<SystemPicker kind="folder" title="Upload a folder to Files" button="Upload" />} />,
  'DW-0481': () => <Win p={pane({ rest: { dropTarget: 'f-documents' } })} overlay={<DragBadge count={3} target="Documents" x={420} y={230} />} />,

  // Browse files
  'DW-1101': () => <Win p={pane()} />,
  'DW-1102': () => <Win p={pane({ folder: 'f-lisbon', sel: ['p-alfama'], prefs: { ...GRID, ...PANEL } })} />,
  'DW-1103': () => <Win p={pane()} overlay={<Menu x={700} y={330} label="List" entries={backgroundMenu(DEFAULT_PREFS, true, true)} onPick={noop} onClose={noop} />} />,
  // Ruled (lead, 2026-11-09, item 3; M3d): the menu on a photo that is downloaded but not kept, Alfama.heic in Lisbon
  // trip, so its Make available offline is there. Harbour.heic is kept (DW-0401), so its menu reads Remove from this PC.
  'DW-1104': () => <Win p={pane({ folder: 'f-lisbon', sel: ['p-alfama'] })} overlay={<RowMenu rowId="p-alfama" label="File" entries={fileMenu([FILES_TREE.find((i) => i.id === 'p-alfama')!])} />} />,
  'DW-1105': () => {
    const hits = FILES_TREE.filter((i) => i.name.toLowerCase().includes('contr')).map((item) => ({ item, where: item.parentId ? `Files › ${FILES_TREE.find((f) => f.id === item.parentId)!.name}` : 'Files' }))
    return <Win body={<SearchView query="Contr" hits={hits} />} />
  },
  'DW-1106': () => <Win p={pane({ folder: 'f-new', rest: { empty: <EmptyFiles />, items: [] } })} />,
  'DW-1107': () => (
    <Win
      p={pane({ rest: { top: <GettingReadyStrip loading={{ done: 18_240, total: 40_112, minutesLeft: 1, thumbnails: 'waiting' }} /> } })}
      status={statusText('Getting your files ready')}
    />
  ),
  'DW-1108': () => <Win offline p={pane({ rest: { offline: true } })} top={offlineBanner} status={statusText('Offline')} />,
  'DW-1109': () => (
    <Win p={pane()} top={<Banner kind="signed-out" copy={bannerCopy('signed-out')} onAction={noop} />} status={statusText('Signed out', 'red')} />
  ),
  'DW-1110': () => (
    <Win p={pane({ folder: 'f-backups', ...BACKUPS, sel: ['f-bk-mail'], rest: { columns: 'backups', canNew: false } })} section="backups" overlay={<ProtectedAlert name="Mail Backup" />} />
  ),

  // Folders, Backups and uploads
  'DW-0420': () => <Win p={pane({ folder: 'f-lisbon' })} />,
  'DW-0420b': () => <Win p={pane({ folder: 'f-archive', rest: { scrollTop: 138 * 36 } })} />,
  'DW-0421': () => <Win p={pane({ folder: 'f-lisbon', prefs: GRID, rest: { videoLength: VIDEO_LENGTH } })} />,
  'DW-0422': () => <Win section="backups" p={pane({ folder: 'f-backups', ...BACKUPS, raw: true, rest: { columns: 'backups', canNew: false } })} />,
  'DW-0423': () => <Win section="backups" p={pane({ folder: 'f-bk-laptop', ...BACKUPS, rest: { canNew: false } })} />,
  'DW-0424': () => <Win section="backups" p={pane({ folder: 'f-bk-mail', ...BACKUPS, rest: { canNew: false } })} top={serviceBanner} />,
  'DW-0425': () => <Win section="backups" p={pane({ folder: 'f-bk-mail', ...BACKUPS, rest: { canNew: false } })} top={serviceBanner} overlay={<ProtectedAlert name="Mail Backup" inside />} />,
  'DW-0426': () => (
    <Win
      p={pane({ folder: 'f-lisbon', rest: { transfers: { 'p-miradouro': { kind: 'uploading', done: 2_100_000, total: 3_600_000 }, 'p-ponte': { kind: 'waiting', done: 0, total: 3_200_000 } } } })}
      status={statusText('Uploading 3 of 5 · 42 MB left')}
    />
  ),
  'DW-0427': () => (
    <Win
      p={pane({ folder: 'f-lisbon', rest: { transfers: { 'p-miradouro': { kind: 'uploading', done: 2_100_000, total: 3_600_000 }, 'p-ponte': { kind: 'waiting', done: 0, total: 3_200_000 } } } })}
      status={statusText('Uploading 3 of 5 · 42 MB left')}
      overlay={
        <UploadsPanel
          summary="3 of 5 · 42 MB left · Continues in the background"
          rows={[
            { id: 'u1', name: 'Scan 12 Oct.pdf', state: 'attention', total: 900_000, note: 'Stopped after 5 tries. Not enough space.' },
            { id: 'u2', name: 'Miradouro.heic', state: 'uploading', done: 2_100_000, total: 3_600_000 },
            { id: 'u3', name: 'Tram 28.mov', state: 'uploading', done: 34_000_000, total: 82_000_000, continuing: true },
            { id: 'u4', name: 'Ponte.heic', state: 'waiting', total: 3_200_000, to: 'Lisbon trip' },
          ]}
        />
      }
    />
  ),
  'DW-0429': () => (
    <Win p={pane({ sel: ['i-contract'] })} overlay={
      <ConflictDialog
        index={1} total={2} name="Contract draft.pdf"
        mine={{ label: 'This PC', keep: 'this PC’s', when: 'Today, 09:12', size: 1_200_000, page: <Page compact title="Contract draft" lines={contractPage.slice(0, 2)} /> }}
        theirs={{ label: 'Anna’s iPad', keep: 'Anna’s iPad’s', when: 'Today, 08:50', size: 1_300_000, page: <Page compact title="Contract draft" lines={contractPage.slice(0, 3)} /> }}
      />
    } />
  ),
}
