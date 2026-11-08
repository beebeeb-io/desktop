// B06's fixtures: the board's Photos (DW-0601 to DW-0611), Sync history (DW-1301 to DW-1306) and Available offline
// (DW-0715) data. Owned by B06 only. Fixtures only: the live sections read DriveSource and the Rust commands.
import type { DriveItem, DriveSource, PhotoEntry } from '@beebeeb/shared/drive-ui'
import { createFixtureSource } from '../fixtures'
import { thumbnailFor } from './files'

export const PHOTOS_NOW = '2026-11-08T12:00:00'
const none = { kind: 'none' } as const

/** What the board shows beside a photo that DriveSource's PhotoEntry does not carry yet (reported as a gap). */
export interface PhotoMeta {
  /** Absent live: the engine doesn't know which device took it. */
  device?: string
  duration?: string
  uploading?: boolean
  camera?: string
  dims?: string
  previewBytes?: number
}

const IPHONE = 'Anna’s iPhone'
const OLD = 'Old iPhone'
const IPAD = 'iPad'

function photo(id: string, name: string, takenAt: string, size: number, device: string, extra: Partial<PhotoMeta> = {}, mime = 'image/heic'): [PhotoEntry, PhotoMeta] {
  const item: DriveItem = {
    id, parentId: 'f-backups', name, isFolder: false, size, modifiedAt: takenAt, mime, shared: none,
    local: 'online-only', hasThumbnail: true, storageCountry: 'DE',
  }
  return [{ item, takenAt }, { device, camera: 'Sony A7R V, 35 mm', dims: '9504 × 6336', previewBytes: 1_000_000, ...extra }]
}

const RAW: [PhotoEntry, PhotoMeta][] = [
  photo('ph-harbour', 'Harbour.heic', '2026-11-08T09:12:00', 30_900_000, IPHONE),
  photo('ph-market', 'Market.heic', '2026-11-08T09:10:00', 4_100_000, IPHONE, { uploading: true }),
  photo('ph-bridge', 'Bridge.heic', '2026-11-08T09:08:00', 3_800_000, IPHONE, { uploading: true }),
  photo('ph-cafe', 'Cafe.heic', '2026-11-08T08:55:00', 3_300_000, IPHONE),
  photo('ph-tram-today', 'Tram.mov', '2026-11-08T08:31:00', 18_000_000, IPHONE, { duration: '0:12' }, 'video/quicktime'),
  photo('ph-sunday', 'Sunday walk.mov', '2026-11-01T14:20:00', 41_000_000, IPHONE, { duration: '0:24' }, 'video/quicktime'),
  photo('ph-sun-2', 'Park.heic', '2026-11-01T13:02:00', 3_100_000, IPAD),
  photo('ph-sun-3', 'Pond.heic', '2026-11-01T12:40:00', 2_900_000, IPAD),
  photo('ph-sun-4', 'Bench.heic', '2026-11-01T12:11:00', 3_500_000, IPHONE),
  photo('ph-lis-1', 'Alfama.heic', '2025-10-12T17:20:00', 3_400_000, OLD),
  photo('ph-lis-2', 'Belém tower.heic', '2025-10-12T16:02:00', 2_900_000, OLD),
  photo('ph-lis-3', 'Miradouro.heic', '2025-10-12T15:40:00', 3_600_000, OLD),
  photo('ph-lis-4', 'Ponte.heic', '2025-10-12T14:31:00', 3_200_000, OLD),
  photo('ph-lis-5', 'Tram 28.mov', '2025-10-12T12:02:00', 82_000_000, OLD, { duration: '0:42' }, 'video/quicktime'),
  photo('ph-lis-6', 'Pastel.heic', '2025-10-12T11:15:00', 2_800_000, IPHONE),
  photo('ph-lis-7', 'Azulejo.heic', '2025-10-12T10:45:00', 3_000_000, IPHONE),
  photo('ph-lis-8', 'Rooftops.heic', '2025-10-12T10:02:00', 3_700_000, IPAD),
]
export const PHOTO_ENTRIES: PhotoEntry[] = RAW.map(([e]) => e)
export const PHOTO_META: Record<string, PhotoMeta> = Object.fromEntries(RAW.map(([e, m]) => [e.item.id, m]))
/** A place name for a day, as the board's "Sunday, 12 October · Lisbon". */
export const DAY_PLACE: Record<string, string> = { '2025-10-12': 'Lisbon' }

export const DEVICE_BACKUPS = [
  { name: IPHONE, photos: 12_418, status: 'is backing up 12 photos · last backup 09:12' },
  { name: OLD, photos: 8_204, status: 'last backup 3 Oct' },
  { name: IPAD, photos: 4_120, status: 'last backup yesterday, 21:40' },
]
export const PHOTO_TOTALS = { photos: 12_430, videos: 842, bytes: 216_000_000_000 }

export function createPhotosSource(entries: PhotoEntry[] = PHOTO_ENTRIES): DriveSource {
  const base = createFixtureSource()
  return {
    ...base,
    async photos() { return { entries } },
    async thumbnail(id) {
      const hit = entries.find((e) => e.item.id === id)
      return hit ? thumbnailFor(hit.item.name) : null
    },
  }
}

// Sync history ---------------------------------------------------------------------------------------------------
export type HistoryScope = 'this-pc' | 'other'
export interface HistoryRow {
  id: string
  name: string
  /** "Uploaded to Contracts", "Downloaded from Anna’s iPhone": what happened and where. */
  what: string
  device: string
  at: string
  scope: HistoryScope
  /** Set on a row that did not sync: why, in plain words. */
  problem?: string
  /** The Beebeeb item the row shows in the window when opened (a trashed file opens Trash). */
  itemId?: string
  trashed?: boolean
  kind: 'file' | 'folder'
}

const PC = 'This PC'
const h = (id: string, name: string, what: string, device: string, at: string, extra: Partial<HistoryRow> = {}): HistoryRow => ({
  id, name, what, device, at, scope: device === PC ? 'this-pc' : 'other', kind: 'file', ...extra,
})

export const HISTORY: HistoryRow[] = [
  h('h1', 'Studio walkthrough.mov', 'Didn’t sync · Video', PC, '2026-11-08T11:52:00', { problem: 'It’s open in another app. Close it and Beebeeb tries again. Nothing was lost.' }),
  h('h2', 'Contract draft.pdf', 'Uploaded to Contracts', PC, '2026-11-08T11:38:00', { itemId: 'i-contract' }),
  h('h3', 'Harbour.heic', 'Backed up to Photos', 'Anna’s iPhone', '2026-11-08T09:12:00', { itemId: 'i-harbour' }),
  h('h4', 'Tram 28.mov', 'Didn’t sync · Lisbon trip', PC, '2026-11-08T08:20:00', { problem: 'Couldn’t reach Beebeeb after 5 tries. Nothing was lost.' }),
  h('h5', 'Budget 2026.xlsx', 'Moved to Finance', PC, '2026-11-07T17:05:00', { itemId: 'i-budget' }),
  h('h6', 'Meeting notes.md', 'Downloaded from Anna’s iPad', PC, '2026-11-07T10:14:00', { itemId: 'i-notes' }),
  h('h7', 'Old invoice.pdf', 'Moved to Trash', PC, '2026-11-07T09:30:00', { trashed: true }),
  h('h8', 'Lisbon trip', 'Updated in a shared folder · Naiches', 'Naiches’s PC', '2026-11-01T18:45:00', { kind: 'folder', itemId: 'f-lisbon' }),
  h('h9', 'Pond.heic', 'Backed up to Photos', 'Anna’s iPad', '2026-11-01T13:02:00', { itemId: 'ph-sun-3' }),
  h('h10', 'Roadmap.pdf', 'Uploaded to Planning', PC, '2026-10-31T16:20:00'),
  h('h11', 'Budget 2026 (2).xlsx', 'Didn’t sync · Finance', PC, '2026-10-31T15:02:00', { problem: 'The name is already in use. Rename it and Beebeeb tries again. Nothing was lost.' }),
  h('h12', 'Contract draft.pdf', 'Edited on Anna’s iPhone', 'Anna’s iPhone', '2026-10-30T11:30:00', { itemId: 'i-contract' }),
  h('h13', 'Brand guide.pdf', 'Downloaded from Anna’s iPhone', PC, '2026-10-30T09:02:00'),
]

// Available offline -----------------------------------------------------------------------------------------------
export interface OfflineEntry { item: DriveItem; bytes: number | null; status: 'Up to date' | 'Downloading' }
const kept = (id: string, name: string, isFolder: boolean, size: number | null, childCount?: number): DriveItem => ({
  id, parentId: null, name, isFolder, size, childCount, modifiedAt: '2026-11-05T10:10:00', shared: none, local: 'available-offline', hasThumbnail: false,
})
export const OFFLINE_ENTRIES: OfflineEntry[] = [
  { item: kept('o-documents', 'Documents', true, null, 48), bytes: 1_400_000_000 - 27_800_000 - 98_304, status: 'Up to date' },
  { item: kept('o-invoices', 'Invoices 2026', true, null, 11), bytes: 27_800_000, status: 'Up to date' },
  { item: kept('o-budget', 'Budget 2026.xlsx', false, 98_304), bytes: 98_304, status: 'Up to date' },
]
