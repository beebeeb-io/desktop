// B04's fixtures: the board's Files tree (DW-0401, DW-0420, DW-0422 to DW-0424) as a DriveSource with real folders,
// so every Files and Backups frame renders its own state headless. Owned by B04 only.
import { DriveSourceError, type DriveItem, type DriveSource, type ItemId, type Listing, type SortDir, type SortKey } from '@beebeeb/shared/drive-ui'
import { createFixtureSource } from '../fixtures'

export const NOW = '2026-11-08T12:00:00'
const at = (day: string, hm = '12:00') => `${day}T${hm}:00`
const none = { kind: 'none' } as const

function dir(id: string, parentId: string | null, name: string, childCount: number, extra: Partial<DriveItem> = {}): DriveItem {
  return { id, parentId, name, isFolder: true, size: null, childCount, modifiedAt: at('2026-11-05', '10:10'), shared: none, local: 'online-only', hasThumbnail: false, ...extra }
}
function doc(id: string, parentId: string | null, name: string, size: number, modifiedAt: string, extra: Partial<DriveItem> = {}): DriveItem {
  return { id, parentId, name, isFolder: false, size, modifiedAt, shared: none, local: 'online-only', hasThumbnail: false, storageCountry: 'DE', ...extra }
}
const PDF = 'application/pdf'
const HEIC = 'image/heic'

const ROOT: DriveItem[] = [
  dir('f-backups', null, 'Backups', 4, { readOnly: true, local: 'downloaded' }),
  dir('f-documents', null, 'Documents', 14, { local: 'downloaded' }),
  dir('f-invoices', null, 'Invoices 2026', 9, { shared: { kind: 'link' }, local: 'downloaded' }),
  dir('f-lisbon', null, 'Lisbon trip', 7, { shared: { kind: 'people', people: [{ name: 'Naiches', initials: 'N' }] } }),
  dir('f-shoot', null, 'Shoot selects', 31, { local: 'available-offline' }),
  doc('i-budget', null, 'Budget 2026.xlsx', 98_304, at('2026-11-07', '09:02'), { mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', local: 'downloaded' }),
  doc('i-contract', null, 'Contract draft.pdf', 1_200_000, at('2026-11-07', '16:42'), { mime: PDF, local: 'online-only', hasThumbnail: true, shared: { kind: 'link' } }),
  doc('i-harbour', null, 'Harbour.heic', 3_100_000, at('2026-10-30', '18:12'), { mime: HEIC, hasThumbnail: true, local: 'available-offline' }),
  doc('i-moodboard', null, 'Moodboard.sketch', 18_400_000, at('2026-10-21', '11:00')),
  doc('i-agreement', null, 'Service agreement.pdf', 640_000, at('2026-10-12', '14:05'), { mime: PDF, local: 'available-offline' }),
  doc('i-notes', null, 'Meeting notes.md', 4_211, at('2026-11-08', '08:41'), { mime: 'text/markdown', local: 'online-only' }),
]

const LISBON: DriveItem[] = [
  dir('f-day1', 'f-lisbon', 'Day 1', 24, { local: 'downloaded' }),
  dir('f-day2', 'f-lisbon', 'Day 2', 31),
  doc('p-alfama', 'f-lisbon', 'Alfama.heic', 3_400_000, at('2026-10-02', '17:20'), { mime: HEIC, hasThumbnail: true, local: 'downloaded' }),
  doc('p-belem', 'f-lisbon', 'Belém tower.heic', 2_900_000, at('2026-10-03', '10:14'), { mime: HEIC, hasThumbnail: true }),
  doc('p-miradouro', 'f-lisbon', 'Miradouro.heic', 3_600_000, at('2026-10-03', '18:40'), { mime: HEIC, hasThumbnail: true }),
  doc('p-ponte', 'f-lisbon', 'Ponte.heic', 3_200_000, at('2026-10-04', '09:31'), { mime: HEIC, hasThumbnail: true }),
  doc('p-tram', 'f-lisbon', 'Tram 28.mov', 82_000_000, at('2026-10-04', '12:02'), { mime: 'video/quicktime', hasThumbnail: true }),
]

const LONG_NAMES = [
  'Quarterly report for the Initlabs B.V. board meeting, final reviewed version 17 (signed by everyone).pdf',
  'Scan 2026-06-14 receipt from the hardware store in Rotterdam about the replacement ladder and paint.jpg',
]
/** DW-0420b: a folder of 212 items, two with names longer than a row. */
export const BIG_FOLDER: DriveItem[] = Array.from({ length: 212 }, (_, i) => {
  const n = i + 1
  if (n === 150) return doc('b-150', 'f-archive', `Archive 150 ${LONG_NAMES[0]}`, 1_800_000, at('2026-06-20', '09:00'), { mime: PDF })
  if (n === 158) return doc('b-158', 'f-archive', `Archive 158 ${LONG_NAMES[1]}`, 2_400_000, at('2026-06-14', '15:30'), { mime: 'image/jpeg', hasThumbnail: true })
  return doc(`b-${n}`, 'f-archive', `Archive ${String(n).padStart(3, '0')}.pdf`, 40_000 + n * 1_731, at('2026-05-01', '10:00'), { mime: PDF })
})

const DOCUMENTS: DriveItem[] = [
  doc('d-initlabs', 'f-documents', 'Contract Initlabs.pdf', 482_133, at('2026-11-06', '15:20'), { mime: PDF, local: 'available-offline' }),
  doc('d-renewal', 'f-documents', 'Contract renewal.docx', 52_400, at('2026-09-18', '10:12'), { mime: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', local: 'downloaded' }),
]

const BACKUPS: DriveItem[] = [
  dir('f-bk-iphone', 'f-backups', 'Anna’s iPhone', 12, { readOnly: true, local: 'online-only' }),
  dir('f-bk-ipad', 'f-backups', 'Anna’s iPad', 8, { readOnly: true }),
  dir('f-bk-laptop', 'f-backups', 'Anna-Laptop', 5, { readOnly: true, local: 'downloaded' }),
  dir('f-bk-old', 'f-backups', 'Old iPhone', 3, { readOnly: true }),
  dir('f-bk-mail', 'f-backups', 'Mail Backup', 3, { readOnly: true, local: 'online-only' }),
]
/** What the Backups columns say beyond the item (DW-0422): the last backup and the status. */
export const BACKUP_META: Record<string, { last: string; status: string; thisPc?: boolean }> = {
  'f-bk-iphone': { last: at('2026-11-08', '09:40'), status: 'Backed up' },
  'f-bk-ipad': { last: at('2026-11-07', '22:15'), status: 'Backed up' },
  'f-bk-laptop': { last: at('2026-11-08', '11:52'), status: 'Backed up', thisPc: true },
  'f-bk-old': { last: at('2026-08-14', '19:03'), status: 'Not signed in' },
  'f-bk-mail': { last: at('2026-11-08', '06:00'), status: 'Managed by Beebeeb' },
}
const BACKUP_CHILDREN: DriveItem[] = [
  dir('f-bkl-desktop', 'f-bk-laptop', 'Desktop', 42, { readOnly: true }),
  dir('f-bkl-documents', 'f-bk-laptop', 'Documents', 318, { readOnly: true }),
  dir('f-bkl-projects', 'f-bk-laptop', 'Projects', 96, { readOnly: true }),
  dir('f-bkl-contacts', 'f-bk-laptop', 'Contacts', 1, { readOnly: true }),
  dir('f-bkl-calendar', 'f-bk-laptop', 'Calendar', 1, { readOnly: true }),
  dir('f-mail-2026', 'f-bk-mail', '2026', 1_204, { readOnly: true }),
  dir('f-mail-2025', 'f-bk-mail', '2025', 3_872, { readOnly: true }),
  dir('f-mail-2024', 'f-bk-mail', '2024', 2_511, { readOnly: true }),
]

export const FILES_TREE: DriveItem[] = [
  ...ROOT, ...DOCUMENTS, ...LISBON, dir('f-archive', null, 'Archive', 212), ...BIG_FOLDER, ...BACKUPS, ...BACKUP_CHILDREN,
  dir('f-new', null, 'New folder', 0),
]

const HIDDEN_AT_ROOT = new Set(['f-archive', 'f-new'])
export const childrenIn = (items: DriveItem[], id: ItemId | null) => items.filter((i) => i.parentId === id && !(id === null && HIDDEN_AT_ROOT.has(i.id)))
export function pathIn(items: DriveItem[], id: ItemId | null): { id: ItemId | null; name: string }[] {
  const chain: DriveItem[] = []
  for (let cur = id ? items.find((i) => i.id === id) : undefined; cur; cur = cur.parentId ? items.find((i) => i.id === cur!.parentId) : undefined) chain.unshift(cur)
  return [{ id: null, name: 'Files' }, ...chain.map((c) => ({ id: c.id, name: c.name }))]
}
/** A listing, synchronously, for frames. */
export function listingOf(folderId: ItemId | null, o: { offline?: boolean; items?: DriveItem[] } = {}): Listing {
  const items = o.items ?? FILES_TREE
  const path = pathIn(items, folderId)
  return { folder: path[path.length - 1], path, items: childrenIn(items, folderId), asOf: o.offline ? at('2026-11-08', '09:12') : NOW, stale: !!o.offline }
}
/** The details a frame shows for one item. */
export function detailsOf(id: ItemId, items: DriveItem[] = FILES_TREE) {
  const item = items.find((i) => i.id === id)!
  return { item, path: pathIn(items, item.parentId), changedOn: item.id === 'p-alfama' ? 'Anna’s iPhone' : 'This PC', versionCount: item.isFolder ? 0 : 3 }
}
/** Video lengths for the grid's badge (DW-0421); the engine does not report them yet. */
export const VIDEO_LENGTH: Record<string, string> = { 'p-tram': '0:42' }

/** A neutral generated image for the fixtures' thumbnails: a soft gradient, hue from the name. */
export function thumbnailFor(name: string): string {
  let h = 0
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${h} 18% 78%)"/><stop offset="1" stop-color="hsl(${(h + 40) % 360} 14% 52%)"/></linearGradient></defs><rect width="320" height="240" fill="url(#g)"/><circle cx="${90 + (h % 120)}" cy="90" r="34" fill="hsl(${h} 10% 92%)" opacity=".55"/></svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

export type FilesScenario = 'default' | 'offline' | 'getting-ready'

/** DW-1108: offline, online-only files are dimmed; the rest open. */
export function createFilesSource(opts: { scenario?: FilesScenario; items?: DriveItem[] } = {}): DriveSource {
  let items = opts.items ?? [...FILES_TREE]
  const base = createFixtureSource()
  const listeners = new Set<Parameters<DriveSource['subscribe']>[0]>()
  const find = (id: ItemId): DriveItem => {
    const hit = items.find((i) => i.id === id)
    if (!hit) throw new DriveSourceError('not-found')
    return hit
  }
  const emit = () => listeners.forEach((l) => l({ kind: 'changed', folderId: null }))
  const childrenOf = (id: ItemId | null) => childrenIn(items, id)
  const pathOf = (id: ItemId | null) => pathIn(items, id)
  let undoSeq = 0
  const trashed = new Map<string, DriveItem[]>()
  const offline = opts.scenario === 'offline'

  return {
    ...base,
    async list(folderId, sort: { key: SortKey; dir: SortDir } = { key: 'name', dir: 'asc' }): Promise<Listing> {
      void sort
      return listingOf(folderId, { offline, items })
    },
    async details(id) {
      const item = find(id)
      return { item, path: pathOf(item.parentId), changedOn: item.id === 'p-alfama' ? 'Anna’s iPhone' : 'This PC', versionCount: item.isFolder ? 0 : 3 }
    },
    async thumbnail(id) {
      const item = items.find((i) => i.id === id)
      if (!item?.hasThumbnail) return null
      return thumbnailFor(item.name)
    },
    async search(query) {
      const q = query.trim().toLowerCase()
      return q ? items.filter((i) => i.name.toLowerCase().includes(q) && !i.id.startsWith('b-')) : []
    },
    async createFolder(parentId, name) {
      if (childrenOf(parentId).some((i) => i.name.toLowerCase() === name.toLowerCase())) throw new DriveSourceError('name-taken')
      const created = dir(`f-created-${items.length}`, parentId, name, 0)
      items = [...items, created]
      emit()
      return created
    },
    async rename(id, name) {
      const cur = find(id)
      if (childrenOf(cur.parentId).some((i) => i.id !== id && i.name.toLowerCase() === name.toLowerCase())) throw new DriveSourceError('name-taken')
      const next = { ...cur, name }
      items = items.map((i) => (i.id === id ? next : i))
      emit()
      return next
    },
    async move(ids, targetId) {
      items = items.map((i) => (ids.includes(i.id) ? { ...i, parentId: targetId } : i))
      emit()
    },
    async duplicate(ids) {
      const copies = ids.map((id) => {
        const src = find(id)
        const dot = src.name.lastIndexOf('.')
        const stem = src.isFolder || dot <= 0 ? src.name : src.name.slice(0, dot)
        const ext = src.isFolder || dot <= 0 ? '' : src.name.slice(dot)
        let name = `${stem} copy${ext}`
        for (let n = 2; childrenOf(src.parentId).some((i) => i.name.toLowerCase() === name.toLowerCase()); n++) name = `${stem} copy ${n}${ext}`
        return { ...src, id: `${src.id}-copy-${items.length}`, name, modifiedAt: new Date().toISOString(), shared: none, local: 'online-only' as const }
      })
      items = [...items, ...copies]
      emit()
      return copies
    },
    async moveToTrash(ids) {
      const token = `undo-${++undoSeq}`
      trashed.set(token, items.filter((i) => ids.includes(i.id)))
      items = items.filter((i) => !ids.includes(i.id))
      emit()
      return { undoToken: token }
    },
    async undoTrash(token) {
      items = [...items, ...(trashed.get(token) ?? [])]
      trashed.delete(token)
      emit()
    },
    async versions(id) {
      const it = find(id)
      return [
        { id: 'v3', createdAt: at('2026-11-07', '16:42'), size: it.size ?? 0, current: true, deviceName: 'This PC' },
        { id: 'v2', createdAt: at('2026-11-07', '11:05'), size: Math.round((it.size ?? 0) * 0.92), current: false, deviceName: 'Anna’s iPhone' },
        { id: 'v1', createdAt: at('2026-10-28', '14:20'), size: Math.round((it.size ?? 0) * 0.82), current: false, deviceName: 'Anna’s iPad' },
      ]
    },
    async setAvailableOffline(id, on) {
      items = items.map((i) => (i.id === id ? { ...i, local: on ? 'available-offline' : 'downloaded' } : i))
      emit()
    },
    async upload() { return { started: 0 } },
    subscribe(listener) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
  }
}
