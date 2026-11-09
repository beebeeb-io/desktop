// Batch M2: the ONE fixture set. A plain browser (the dev server, headless screenshots, the map audit) gets this source,
// and it is made of the same fixture modules the frame registry renders: B04's Files tree, B05's shares, search and
// Trash, B06's photos, B07's open files, B08's Office documents. Names, sizes and counts are the board's, so a live
// screen and its frame show the same rows. `?scenario=` picks a state the global frames need (see ../fixtures.ts).
import type { DriveEvent, DriveSource, ItemId, StorageSummary, SyncSummary, TrashEntry } from '@beebeeb/shared/drive-ui'
import type { Scenario } from '../fixtures'
import { createFilesSource } from './files'
import { createSharedSource, SHARED_IDS, TRASH } from './shared'
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
  const files = createFilesSource({ scenario: scenario === 'offline' || scenario === 'rename-fails' ? scenario : 'default', items: empty ? [] : undefined })
  // `no-shares` (DW-0508 to DW-0510): nothing shared yet, while Files keeps DW-0401's rows (Files › leads somewhere).
  const shared = createSharedSource({ empty: empty || scenario === 'no-shares' })
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

  // One Trash for every source: what Files moves there shows in Trash and its count, Undo and Restore take it back, and
  // a move or a delete sends `removed` per item as the engine does (src-tauri drive_v2 commands.rs).
  let trash: TrashEntry[] = empty ? [] : [...TRASH]
  const moves = new Map<string, ItemId[]>()
  const listeners = new Set<(e: DriveEvent) => void>()
  const send = (e: DriveEvent) => listeners.forEach((l) => l(e))
  const stamp = () => {
    const d = new Date(), two = (n: number) => String(n).padStart(2, '0')
    return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}:00`
  }
  const leaveTrash = (ids: ItemId[]) => { trash = trash.filter((t) => !ids.includes(t.item.id)) }

  return {
    ...files,
    kind: 'fixtures',
    details: async (id) => {
      if (officeAlias.has(id)) return files.details(id)
      // B05's shares (Shared › Details) are the shared fixtures' own items.
      if (SHARED_IDS.has(id)) return shared.details(id)
      return owner(id).details(id)
    },
    thumbnail: (id, size) => owner(id).thumbnail(id, size),
    versions: (id) => (SHARED_IDS.has(id) ? shared.versions(id) : owner(id).versions(idFor(id))),
    readText: (id) => owner(id).readText(idFor(id)),
    saveText: (id, text, base) => owner(id).saveText(idFor(id), text, base),
    readBytes: (id) => owner(id).readBytes(idFor(id)),
    saveBytes: (id, bytes, base) => owner(id).saveBytes(idFor(id), bytes, base),
    // B05's search hits (DW-0520 on); a new account (DW-0114) has nothing to find.
    search: async (q) => (empty ? [] : shared.search(q)),
    shared: () => shared.shared(),
    trash: async () => trash,
    async moveToTrash(ids) {
      const found = await Promise.all(ids.map((id) => files.details(id).catch(() => null)))
      const moved = await files.moveToTrash(ids)
      const entries = found.flatMap((d) => (d ? [{ item: d.item, trashedAt: stamp(), originalPath: d.path.slice(1).map((c) => c.name).join(' › ') || 'Files' }] : []))
      trash = [...entries, ...trash]
      moves.set(moved.undoToken, ids)
      ids.forEach((id) => send({ kind: 'removed', id }))
      return moved
    },
    async undoTrash(token) {
      await files.undoTrash(token)
      leaveTrash(moves.get(token) ?? [])
      moves.delete(token)
    },
    async restoreFromTrash(ids) {
      leaveTrash(ids)
      // A move from Files goes back once all of it has left Trash.
      for (const [token, moved] of moves) {
        if (moved.every((id) => !trash.some((t) => t.item.id === id))) { moves.delete(token); await files.undoTrash(token) }
      }
    },
    async deletePermanently(ids) {
      leaveTrash(ids)
      for (const [token, moved] of moves) if (moved.every((id) => !trash.some((t) => t.item.id === id))) moves.delete(token)
      ids.forEach((id) => send({ kind: 'removed', id }))
    },
    photos: (cursor) => photos.photos(cursor),
    storage: async () => boardStorage(scenario),
    sync: async () => boardSync(scenario),
    subscribe(listener) {
      listeners.add(listener)
      const stops = [files, photos, open, office].map((s) => s.subscribe(listener))
      return () => { listeners.delete(listener); stops.forEach((stop) => stop()) }
    },
  }
}
