// Batch B06's frames (design/desktop-v2/BATCHES.md): Photos (13) and This PC: Sync history, Available offline (9).
// Each renders the real section views against B06's fixtures (source/fixtures/photos.ts). Owned by B06 only.
import type { ReactElement, ReactNode } from 'react'
import { DriveSourceProvider } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Sidebar } from '../chrome/Sidebar'
import { StatusLine } from '../chrome/StatusLine'
import { ToastView, type ToastMessage } from '../chrome/Toast'
import { STORAGE_240, SYNC_OK } from './FrameWindow'
import type { SectionId } from '../routes'
import { PhotosView, type PhotosViewProps } from '../sections/photos/PhotosView'
import { PhotoViewer, type ViewerProps } from '../sections/photos/Viewer'
import { DetailsDialog, ShareDialog, SourcesDialog } from '../sections/photos/Dialogs'
import { groupByDay, idOf, type PhotoCell } from '../sections/photos/model'
import { HistoryView, type HistoryViewProps } from '../sections/sync-history/HistoryView'
import { OfflineView, type OfflineRow } from '../sections/available-offline/OfflineView'
import {
  createPhotosSource, DAY_PLACE, DEVICE_BACKUPS, HISTORY, OFFLINE_ENTRIES, PHOTO_ENTRIES, PHOTO_META, PHOTO_TOTALS, PHOTOS_NOW,
} from '../source/fixtures/photos'

const SRC = createPhotosSource()
const NOW = new Date(PHOTOS_NOW)
const GB = 1024 ** 3
const STORAGE = { ...STORAGE_240, usedBytes: 240 * GB, filesBytes: 24 * GB, photosBytes: 216 * GB, backupsBytes: 0 }

const CELLS: PhotoCell[] = PHOTO_ENTRIES.map((entry) => ({ entry, meta: PHOTO_META[entry.item.id] }))
const ORDERED = groupByDay(CELLS).flatMap((d) => d.cells)
const at = (id: string) => ORDERED.findIndex((c) => idOf(c) === id)
const DEVICES = DEVICE_BACKUPS.map((d) => d.name)
const BACKUPS = DEVICE_BACKUPS.map((d) => ({ name: d.name, status: d.status }))

function Win({ section, body, overlay, toast, counts }: { section: SectionId; body: ReactNode; overlay?: ReactNode; toast?: ToastMessage; counts?: Partial<Record<SectionId, number>> }): ReactElement {
  return (
    <DriveSourceProvider source={SRC}>
      <div data-frame-window className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
        <CaptionBarView title={section === 'photos' ? 'Photos - Beebeeb' : section === 'sync-history' ? 'Sync history - Beebeeb' : 'Available offline - Beebeeb'} />
        <div className="flex min-h-0 flex-1">
          {/* The board draws Shared 1 and Trash 12 in every B06 frame, as in DW-0401 (M3d). */}
          <Sidebar active={section} onGo={() => {}} storage={STORAGE} accountName="Anna" counts={{ shared: 1, trash: 12, ...counts }} />
          <main className="flex min-w-0 flex-1 flex-col">{body}</main>
        </div>
        <StatusLine sync={SYNC_OK} />
        {toast && <div className="pointer-events-none absolute inset-x-0 bottom-10 z-40 flex justify-center"><ToastView message={toast} /></div>}
        {overlay}
      </div>
    </DriveSourceProvider>
  )
}

const grid = (o: Partial<PhotosViewProps> = {}) => (
  <PhotosView
    cells={CELLS} now={NOW} across={7} devices={DEVICES} device={null} totals={PHOTO_TOTALS} backups={BACKUPS} places={DAY_PLACE}
    selecting={false} selected={new Set()} canUpload {...o}
  />
)
const photos = (body: ReactNode, overlay?: ReactNode, toast?: ToastMessage) => <Win section="photos" body={body} overlay={overlay} toast={toast} />
const viewer = (id: string, o: Partial<ViewerProps> = {}) => <PhotoViewer cells={ORDERED} index={at(id)} now={NOW} mode="preview" {...o} />
const harbour = CELLS.find((c) => idOf(c) === 'ph-harbour')!

const history = (o: Partial<HistoryViewProps> = {}) => (
  <Win section="sync-history" counts={{ 'sync-history': 3 }} body={<HistoryView rows={HISTORY} now={NOW} filter="all" query="" {...o} />} />
)

const OFFLINE_ROWS: OfflineRow[] = OFFLINE_ENTRIES.map((e) => ({
  id: e.item.id, name: e.item.name, isFolder: e.item.isFolder, items: e.item.childCount, bytes: e.bytes, mime: e.item.mime, status: e.status,
}))
const offline = (rows: OfflineRow[], menuOn?: string) => <Win section="available-offline" body={<OfflineView rows={rows} menuOn={menuOn} />} />

export const FRAMES: FrameRegistry = {
  'DW-0601': () => photos(grid()),
  'DW-0602': () => photos(grid({ selecting: true, selected: new Set(['ph-harbour', 'ph-cafe', 'ph-sun-4']) })),
  'DW-0603': () => photos(grid({ acrossOpen: true })),
  'DW-0604': () => photos(grid(), (
    <SourcesDialog
      devices={[{ name: 'Anna’s iPhone', photos: 12_418, on: true }, { name: 'iPad', photos: 4_120, on: true }, { name: 'Old iPhone', photos: 8_204, on: true }]}
      folders={[{ name: 'Lisbon trip', items: 212, note: 'with its subfolders', on: true }]}
    />
  )),
  'DW-0605': () => photos(viewer('ph-harbour')),
  'DW-0605b': () => photos(viewer('ph-harbour', { mode: 'original', progress: { done: 12_400_000, total: 30_900_000 } })),
  'DW-0606': () => photos(viewer('ph-harbour', { moreOpen: true })),
  'DW-0607': () => photos(viewer('ph-harbour'), <ShareDialog cell={harbour} />),
  'DW-0608': () => photos(viewer('ph-harbour', { kept: true, keeping: true })),
  'DW-0609': () => photos(viewer('ph-harbour'), <DetailsDialog cell={harbour} now={NOW} />),
  'DW-0610': () => photos(viewer('ph-market'), undefined, { kind: 'undo', text: 'Moved to Trash' }),
  'DW-0611': () => photos(viewer('ph-sunday', { referencePlayer: true })),
  'DW-0116': () => photos(grid({ cells: [], devices: [], backups: [], totals: null, canUpload: false })),

  'DW-1301': () => history(),
  'DW-1302': () => history({ filter: 'this-pc' }),
  'DW-1303': () => history({ filter: 'other' }),
  'DW-0428': () => history({ filter: 'problems' }),
  'DW-1304': () => history({ filter: 'other', query: 'contract' }),
  'DW-1305': () => <Win section="sync-history" body={<HistoryView rows={[]} now={NOW} filter="all" query="" />} />,

  'DW-0715': () => offline(OFFLINE_ROWS),
  'DW-1306': () => offline(OFFLINE_ROWS, 'o-invoices'),
  'DW-0120': () => offline([]),
}
