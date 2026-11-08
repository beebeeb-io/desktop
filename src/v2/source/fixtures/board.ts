// Batch M2: the ONE fixture set. A plain browser (the dev server, headless screenshots, the map audit) gets this source,
// and it is made of the same fixture modules the frame registry renders: B04's Files tree, B05's shares, search and
// Trash, B06's photos, B07's open files, B08's Office documents. Names, sizes and counts are the board's, so a live
// screen and its frame show the same rows. `?scenario=` picks a state the global frames need (see ../fixtures.ts).
import type { DriveSource, ItemId, StorageSummary, SyncSummary } from '@beebeeb/shared/drive-ui'
import type { Scenario } from '../fixtures'
import { createFilesSource } from './files'
import { createSharedSource } from './shared'
import { createPhotosSource, PHOTO_ENTRIES } from './photos'
import { createOpenSource, OPEN_ITEMS } from './open'
import { createOfficeSource, OFFICE_ITEMS } from './office'

const GB = 1024 ** 3

/** DW-0401's meter: 240 GB of 1 TB, Files 24 GB, Photos 216 GB; DW-0103 at 950 GB; DW-0104 full; DW-0114 empty. */
export function boardStorage(scenario: Scenario): StorageSummary {
  const used = scenario === 'full' ? 1024 : scenario === 'nearly-full' ? 950 : scenario === 'empty' ? 0 : 240
  return { usedBytes: used * GB, quotaBytes: 1024 * GB, filesBytes: (scenario === 'empty' ? 0 : 24) * GB, photosBytes: (scenario === 'empty' ? 0 : 216) * GB, backupsBytes: 0 }
}

/** DW-0401's status line: Up to date · 4 on this PC · 3 available offline · 4 online only. */
export function boardSync(scenario: Scenario): SyncSummary {
  if (scenario === 'empty') return { phase: 'up-to-date', onDevice: 0, availableOffline: 0, onlineOnly: 0, pending: 0 }
  return {
    phase: scenario === 'offline' ? 'offline' : scenario === 'syncing' ? 'syncing' : 'up-to-date',
    onDevice: 4, availableOffline: 3, onlineOnly: 4, pending: scenario === 'syncing' ? 7 : 0,
  }
}

export function createBoardSource(scenario: Scenario = 'default'): DriveSource {
  const empty = scenario === 'empty'
  const files = createFilesSource({ scenario: scenario === 'offline' ? 'offline' : 'default', items: empty ? [] : undefined })
  const shared = createSharedSource({ empty })
  const photos = createPhotosSource(empty ? [] : PHOTO_ENTRIES)
  const open = createOpenSource()
  const office = createOfficeSource()
  const openIds = new Set(OPEN_ITEMS.map((i) => i.id))
  const officeIds = new Set(OFFICE_ITEMS.map((i) => i.id))
  const photoIds = new Set(PHOTO_ENTRIES.map((e) => e.item.id))
  // A Files row that is one of B08's documents (Budget 2026.xlsx) opens B08's document.
  const officeAlias = new Map<ItemId, ItemId>([['i-budget', 'x-budget']])
  const owner = (id: ItemId): DriveSource => (openIds.has(id) ? open : officeIds.has(id) || officeAlias.has(id) ? office : photoIds.has(id) ? photos : files)
  const idFor = (id: ItemId): ItemId => officeAlias.get(id) ?? id

  return {
    ...files,
    kind: 'fixtures',
    details: async (id) => {
      if (officeAlias.has(id)) return files.details(id)
      return owner(id).details(id)
    },
    thumbnail: (id, size) => owner(id).thumbnail(id, size),
    versions: (id) => owner(id).versions(idFor(id)),
    readText: (id) => owner(id).readText(idFor(id)),
    saveText: (id, text, base) => owner(id).saveText(idFor(id), text, base),
    readBytes: (id) => owner(id).readBytes(idFor(id)),
    saveBytes: (id, bytes, base) => owner(id).saveBytes(idFor(id), bytes, base),
    search: (q) => shared.search(q),
    shared: () => shared.shared(),
    trash: () => shared.trash(),
    restoreFromTrash: (ids) => shared.restoreFromTrash(ids),
    deletePermanently: (ids) => shared.deletePermanently(ids),
    photos: (cursor) => photos.photos(cursor),
    storage: async () => boardStorage(scenario),
    sync: async () => boardSync(scenario),
    subscribe(listener) {
      const stops = [files, photos, open, office].map((s) => s.subscribe(listener))
      return () => stops.forEach((stop) => stop())
    },
  }
}
