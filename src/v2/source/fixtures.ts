// The fixtures DriveSource: the board's own data (DW-0401), so every frame builds and screenshots headless
// with no Tauri, no network and no account. `?scenario=` picks a state the global frames need:
//   default | empty (DW-0114) | offline (DW-0101) | nearly-full (DW-0103) | full (DW-0104) | syncing (DW-1203)
// Batches add fixture data in src/v2/source/fixtures/<area>.ts (their own files), never by editing this one's rows.

import {
  DriveSourceError,
  type DriveItem,
  type DriveSource,
  type ItemId,
  type Listing,
  type SortDir,
  type SortKey,
  type StorageSummary,
  type SyncSummary,
} from '@beebeeb/shared/drive-ui'

export type Scenario = 'default' | 'empty' | 'offline' | 'nearly-full' | 'full' | 'syncing'

const GB = 1024 ** 3
const NOW = '2026-11-08T10:00:00Z'

function folder(id: string, name: string, childCount: number, extra: Partial<DriveItem> = {}): DriveItem {
  return {
    id, parentId: null, name, isFolder: true, size: null, childCount, modifiedAt: NOW,
    shared: { kind: 'none' }, local: 'online-only', hasThumbnail: false, ...extra,
  }
}

function file(id: string, name: string, size: number, modifiedAt: string, extra: Partial<DriveItem> = {}): DriveItem {
  return {
    id, parentId: null, name, isFolder: false, size, modifiedAt,
    shared: { kind: 'none' }, local: 'online-only', hasThumbnail: false, storageCountry: 'DE', ...extra,
  }
}

/** DW-0401's list: Backups first, then the folders, then the files by name. */
export const ROOT_ITEMS: DriveItem[] = [
  folder('f-backups', 'Backups', 2, { readOnly: true }),
  folder('f-documents', 'Documents', 14, { local: 'downloaded' }),
  folder('f-invoices', 'Invoices 2026', 9, { shared: { kind: 'link' } }),
  folder('f-lisbon', 'Lisbon trip', 86, {
    shared: { kind: 'people', people: [{ name: 'Naiches', initials: 'N' }] },
  }),
  folder('f-shoot', 'Shoot selects', 31, { local: 'available-offline' }),
  file('i-contract', 'Contract Initlabs.pdf', 482_133, '2026-11-06T15:20:00Z', { mime: 'application/pdf', local: 'available-offline', hasThumbnail: true }),
  file('i-budget', 'Budget 2027.xlsx', 38_912, '2026-11-07T09:02:00Z', { mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', local: 'downloaded' }),
  file('i-notes', 'Meeting notes.md', 4_211, '2026-11-08T08:41:00Z', { mime: 'text/markdown', local: 'available-offline' }),
  file('i-harbour', 'Harbour.jpg', 6_420_118, '2026-10-30T18:12:00Z', { mime: 'image/jpeg', hasThumbnail: true }),
  file('i-pitch', 'Pitch.pptx', 2_912_004, '2026-10-21T11:00:00Z', { mime: 'application/vnd.openxmlformats-officedocument.presentationml.presentation' }),
  file('i-passport', 'Passport scan.png', 1_204_551, '2026-09-02T07:30:00Z', { mime: 'image/png', local: 'downloaded', hasThumbnail: true }),
]

function sortItems(items: DriveItem[], key: SortKey, dir: SortDir): DriveItem[] {
  const sign = dir === 'asc' ? 1 : -1
  const by = (a: DriveItem, b: DriveItem): number => {
    if (key === 'modified') return a.modifiedAt.localeCompare(b.modifiedAt)
    if (key === 'size') return (a.size ?? 0) - (b.size ?? 0)
    if (key === 'kind') return (a.mime ?? '').localeCompare(b.mime ?? '')
    return a.name.localeCompare(b.name)
  }
  // Folders first, Backups first among them (DW-0401 Rules), whatever the sort.
  return [...items].sort((a, b) => {
    if (a.isFolder !== b.isFolder) return a.isFolder ? -1 : 1
    if (a.id === 'f-backups') return -1
    if (b.id === 'f-backups') return 1
    return sign * by(a, b)
  })
}

export function createFixtureSource(scenario: Scenario = 'default'): DriveSource {
  let items = scenario === 'empty' ? [] : [...ROOT_ITEMS]
  const listeners = new Set<Parameters<DriveSource['subscribe']>[0]>()
  const find = (id: ItemId): DriveItem => {
    const hit = items.find((i) => i.id === id)
    if (!hit) throw new DriveSourceError('not-found')
    return hit
  }
  const changed = () => listeners.forEach((l) => l({ kind: 'changed', folderId: null }))
  const storage: StorageSummary = {
    usedBytes: scenario === 'full' ? 1024 * GB : scenario === 'nearly-full' ? 930 * GB : 240 * GB,
    quotaBytes: 1024 * GB,
    filesBytes: 24 * GB,
    photosBytes: 216 * GB,
    backupsBytes: 0,
  }
  const sync: SyncSummary = {
    phase: scenario === 'offline' ? 'offline' : scenario === 'syncing' ? 'syncing' : 'up-to-date',
    onDevice: 4,
    availableOffline: 3,
    onlineOnly: 4,
    pending: scenario === 'syncing' ? 7 : 0,
  }
  let undoSeq = 0
  const trashed = new Map<string, DriveItem[]>()

  return {
    kind: 'fixtures',
    capabilities: { localCopies: true, revealInFileManager: true, office: true, nativePickers: true },

    async list(folderId, sort = { key: 'name', dir: 'asc' }): Promise<Listing> {
      const inFolder = folderId == null ? items : []
      const folderItem = folderId == null ? null : items.find((i) => i.id === folderId)
      const crumb = { id: folderId, name: folderItem?.name ?? 'Files' }
      return {
        folder: crumb,
        path: folderId == null ? [crumb] : [{ id: null, name: 'Files' }, crumb],
        items: sortItems(inFolder, sort.key, sort.dir),
        asOf: scenario === 'offline' ? '2026-11-08T07:12:00Z' : NOW,
        stale: scenario === 'offline',
      }
    },
    async details(id) {
      return { item: find(id), path: [{ id: null, name: 'Files' }], changedOn: 'This PC', versionCount: 3 }
    },
    async thumbnail() { return null },
    async versions(id) {
      const item = find(id)
      return [
        { id: 'v3', createdAt: item.modifiedAt, size: item.size ?? 0, current: true, deviceName: 'This PC' },
        { id: 'v2', createdAt: '2026-10-01T10:00:00Z', size: item.size ?? 0, current: false, deviceName: 'iPhone' },
        { id: 'v1', createdAt: '2026-09-01T10:00:00Z', size: item.size ?? 0, current: false, deviceName: 'MacBook' },
      ]
    },
    async search(query) {
      const q = query.trim().toLowerCase()
      return q ? items.filter((i) => i.name.toLowerCase().includes(q)) : []
    },
    async recent(limit = 10) {
      return [...items].filter((i) => !i.isFolder).sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt)).slice(0, limit)
    },
    async shared() {
      return items
        .filter((i) => i.shared.kind !== 'none')
        .map((item) => ({
          item,
          direction: 'by-me' as const,
          people: item.shared.kind === 'people' ? item.shared.people.map((p) => ({ name: p.name, role: 'editor' as const })) : [],
          hasLink: item.shared.kind === 'link',
        }))
    },
    async photos() {
      return { entries: items.filter((i) => i.mime?.startsWith('image/')).map((item) => ({ item, takenAt: item.modifiedAt })) }
    },
    async trash() { return [] },
    async storage() { return storage },
    async sync() { return sync },
    subscribe(listener) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },

    async createFolder(_parentId, name) {
      if (items.some((i) => i.name.toLowerCase() === name.toLowerCase())) throw new DriveSourceError('name-taken')
      const created = folder(`f-new-${items.length}`, name, 0)
      items = [...items, created]
      changed()
      return created
    },
    async rename(id, name) {
      if (items.some((i) => i.id !== id && i.name.toLowerCase() === name.toLowerCase())) throw new DriveSourceError('name-taken')
      const next = { ...find(id), name }
      items = items.map((i) => (i.id === id ? next : i))
      changed()
      return next
    },
    async move(ids) {
      items = items.filter((i) => !ids.includes(i.id))
      changed()
    },
    async duplicate(ids) {
      const copies = ids.map((id) => {
        const src = find(id)
        const dot = src.name.lastIndexOf('.')
        const name = dot > 0 ? `${src.name.slice(0, dot)} (copy)${src.name.slice(dot)}` : `${src.name} (copy)`
        return { ...src, id: `${src.id}-copy-${items.length}`, name }
      })
      items = [...items, ...copies]
      changed()
      return copies
    },
    async moveToTrash(ids) {
      const token = `undo-${++undoSeq}`
      trashed.set(token, items.filter((i) => ids.includes(i.id)))
      items = items.filter((i) => !ids.includes(i.id))
      changed()
      return { undoToken: token }
    },
    async undoTrash(token) {
      items = [...items, ...(trashed.get(token) ?? [])]
      trashed.delete(token)
      changed()
    },
    async restoreFromTrash() {},
    async deletePermanently() {},
    async restoreVersion() {},
    async upload() { return { started: 0 } },
    async open() {},
    async readText(id) {
      find(id)
      return { text: '# Meeting notes\n\n- Desktop v2 board review\n- Alpha date\n', versionId: 'v3' }
    },
    async saveText(_id, text) { return { text, versionId: `v${Date.now()}` } },
    async readBytes(id) {
      find(id)
      return { bytes: new Uint8Array(), versionId: 'v3' }
    },
    async saveBytes(_id, bytes) { return { bytes, versionId: `v${Date.now()}` } },
    async setAvailableOffline(id, on) {
      items = items.map((i) => (i.id === id ? { ...i, local: on ? 'available-offline' : 'downloaded' } : i))
      changed()
    },
    async revealInFileManager() {},
  }
}

export function scenarioFromSearch(search: string): Scenario {
  const value = new URLSearchParams(search).get('scenario')
  const known: Scenario[] = ['default', 'empty', 'offline', 'nearly-full', 'full', 'syncing']
  return known.includes(value as Scenario) ? (value as Scenario) : 'default'
}
