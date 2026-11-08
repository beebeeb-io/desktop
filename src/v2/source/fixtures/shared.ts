// B05's fixture data: Shared (DW-0501 to DW-0515), Search (DW-0801 to DW-0803) and Trash (DW-0747 to DW-0749b,
// DW-0117), in the board's words. Owned by B05 only.
import type { DriveItem, DriveSource, ShareEntry, TrashEntry } from '@beebeeb/shared/drive-ui'
import { createFilesSource } from './files'

export const B05_NOW = '2026-11-08T12:00:00'
const KB = 1024
const MB = 1024 ** 2

function item(id: string, name: string, o: Partial<DriveItem> = {}): DriveItem {
  return {
    id, parentId: null, name, isFolder: false, size: null, modifiedAt: '2026-11-02T09:30:00',
    shared: { kind: 'none' }, local: 'online-only', hasThumbnail: false, storageCountry: 'DE', ...o,
  }
}

// ── Shared: with me ──────────────────────────────────────────────────────────

export interface Invitation { id: string; name: string; from: string; isFolder: boolean }
export interface WithMeRow { id: string; name: string; isFolder: boolean; owner: string; items?: number; size?: number | null; modifiedAt?: string }

export const INVITATIONS: Invitation[] = [{ id: 'inv-autumn', name: 'Autumn shoot', from: 'Lotte de Wit', isFolder: true }]

export const WITH_ME: WithMeRow[] = [
  { id: 'sw-studio', name: 'Studio assets', isFolder: true, owner: 'Jonas Meier', items: 34, modifiedAt: '2026-11-07T16:20:00' },
  { id: 'sw-brand', name: 'Brand review.pdf', isFolder: false, owner: 'Priya Shah', size: 2.4 * MB, modifiedAt: '2026-11-06T11:05:00' },
  { id: 'sw-harbour', name: 'Harbourlight shoot', isFolder: true, owner: 'Mara Jansen', items: 118, modifiedAt: '2026-11-03T08:40:00' },
  { id: 'sw-rates', name: 'Rates 2026.xlsx', isFolder: false, owner: 'Tobias Lang', size: 62 * KB, modifiedAt: '2026-10-29T14:10:00' },
]

// ── Shared: by me ────────────────────────────────────────────────────────────

export interface PeopleShare { id: string; name: string; isFolder: boolean; people: string[]; pending?: boolean; items?: number; size?: number | null }
export interface LinkShare { id: string; name: string; expiresAt: string | null; password: boolean; size?: number | null }

export const BY_ME_PEOPLE: PeopleShare[] = [
  { id: 'sb-lisbon', name: 'Lisbon trip', isFolder: true, people: ['Jonas Meier', 'Mia de Vries'], items: 212 },
  { id: 'sb-invoices', name: 'Invoices 2026', isFolder: true, people: ['Priya Shah'], items: 41 },
  { id: 'sb-contract', name: 'Contract draft.pdf', isFolder: false, people: ['Sam Okafor'], pending: true, size: 412 * KB },
]

export const BY_ME_LINKS: LinkShare[] = [
  { id: 'sl-budget', name: 'Budget 2026.xlsx', expiresAt: '2026-11-20T12:00:00', password: false, size: 88 * KB },
  { id: 'sl-harbour', name: 'Harbour.heic', expiresAt: '2026-12-05T12:00:00', password: true, size: 3.1 * MB },
  { id: 'sl-quote', name: 'Quote 2025.pdf', expiresAt: '2026-10-03T12:00:00', password: false, size: 140 * KB },
]

// ── Shared: requests ─────────────────────────────────────────────────────────

export interface FileRequestRow {
  id: string
  title: string
  files: number
  /** "12 of 50 files" when the request caps the number of files. */
  maxFiles?: number
  into?: string
  expiresAt?: string | null
  closed?: boolean
}

export const REQUESTS: FileRequestRow[] = [
  { id: 'rq-contracts', title: 'Signed contracts', files: 3, into: 'Documents', expiresAt: null },
  { id: 'rq-selects', title: 'Shoot selects', files: 12, maxFiles: 50, expiresAt: '2026-11-14T12:00:00' },
  { id: 'rq-q3', title: 'Invoices for Q3', files: 5, closed: true },
  { id: 'rq-logo', title: 'Logo files', files: 0, expiresAt: '2026-10-03T12:00:00' },
]

// ── People with access, a person, the safety number ──────────────────────────

export type Access = 'Can view' | 'Can edit'
export interface Member { id: string; name: string; email: string; you?: boolean; owner?: boolean; access?: Access; invited?: boolean; compared?: boolean; mismatch?: boolean }

export const LISBON = { id: 'sb-lisbon', name: 'Lisbon trip', items: 212 }
export const LISBON_PEOPLE: Member[] = [
  { id: 'm-anna', name: 'Anna Visser', email: 'anna@visser.studio', you: true, owner: true },
  { id: 'm-jonas', name: 'Jonas Meier', email: 'jonas@northwind.studio', access: 'Can edit' },
  { id: 'm-mia', name: 'Mia de Vries', email: 'mia@devries.nl', access: 'Can view', invited: true },
]

/** Twelve groups of five digits, as both devices show them. */
export const SAFETY_NUMBER = [
  '37 015', '82 664', '10 293', '55 418', '90 772', '24 806', '61 359', '07 441', '48 120', '93 587', '16 034', '72 965',
].map((g) => g.replace(' ', ''))

// ── Search ───────────────────────────────────────────────────────────────────

export interface SearchRow { item: DriveItem; where: string }

export const RECENT_SEARCHES = ['contract', 'lisbon', 'invoice 2026']

export const SEARCH_ITEMS: SearchRow[] = [
  { item: item('s-contract-draft', 'Contract draft.pdf', { size: 412 * KB, modifiedAt: '2026-11-07T10:12:00', local: 'downloaded' }), where: 'Files' },
  { item: item('s-contract-2025', 'Contract signed 2025.pdf', { size: 1.2 * MB, modifiedAt: '2025-12-18T15:44:00', local: 'online-only' }), where: 'Documents › Legal' },
  { item: item('s-contractors', 'Contractors', { isFolder: true, childCount: 9, modifiedAt: '2026-09-21T09:00:00', local: 'available-offline' }), where: 'Documents' },
  { item: item('s-lisbon', 'Lisbon trip', { isFolder: true, childCount: 212, modifiedAt: '2026-10-12T19:30:00' }), where: 'Files' },
  { item: item('s-harbour', 'Harbour.heic', { size: 3.1 * MB, mime: 'image/heic', modifiedAt: '2026-10-11T18:02:00' }), where: 'Photos' },
  { item: item('s-invoice', 'Invoice 2026-031.pdf', { size: 96 * KB, modifiedAt: '2026-10-31T08:15:00' }), where: 'Documents › Invoices 2026' },
]

export const SEARCH_LOADING = { done: 18_240, total: 40_112 }

// ── Trash ────────────────────────────────────────────────────────────────────

const T = (id: string, name: string, trashedAt: string, originalPath: string, o: Partial<DriveItem> = {}): TrashEntry => ({
  item: item(id, name, o), trashedAt, originalPath,
})

/** Newest deleted first; 12 items, 240 MB in all (the six the board names, then six more). */
export const TRASH: TrashEntry[] = [
  T('t-old-invoices', 'Old invoices', '2026-11-08T09:40:00', 'Documents', { isFolder: true, childCount: 18, size: 64 * MB }),
  T('t-contract-old', 'Contract old.pdf', '2026-11-07T17:22:00', 'Documents', { size: 380 * KB }),
  T('t-budget-2025', 'Budget 2025.xlsx', '2026-11-05T11:03:00', 'Documents › Finance', { size: 92 * KB }),
  T('t-img-2044', 'IMG_2044.HEIC', '2026-11-02T20:18:00', 'Photos', { size: 3.4 * MB, mime: 'image/heic' }),
  T('t-meeting', 'Meeting notes.pdf', '2026-10-29T08:55:00', 'Files', { size: 210 * KB }),
  T('t-drafts', 'Drafts', '2026-10-27T16:10:00', 'Files', { isFolder: true, childCount: 6, size: 12 * MB }),
  T('t-mood', 'Moodboard v1.png', '2026-10-24T13:31:00', 'Lisbon trip', { size: 8.2 * MB, mime: 'image/png' }),
  T('t-pitch', 'Pitch deck old.pptx', '2026-10-21T10:02:00', 'Documents', { size: 24 * MB }),
  T('t-scan', 'Scan 0912.pdf', '2026-10-18T09:44:00', 'Files', { size: 1.6 * MB }),
  T('t-clip', 'Harbour clip.mov', '2026-10-15T19:20:00', 'Photos', { size: 118 * MB, mime: 'video/quicktime' }),
  T('t-notes', 'Notes.txt', '2026-10-13T07:58:00', 'Files', { size: 4 * KB }),
  T('t-export', 'Export 2024.zip', '2026-10-11T12:12:00', 'Backups', { size: 6.8 * MB }),
]

export const TRASH_TOTAL_BYTES = 240 * MB

// ── The fixtures DriveSource for B05 ─────────────────────────────────────────

function shareEntries(): ShareEntry[] {
  const withMe: ShareEntry[] = WITH_ME.map((r) => ({
    item: item(r.id, r.name, { isFolder: r.isFolder, size: r.size ?? null, childCount: r.items, readOnly: true, shared: { kind: 'people', people: [] } }),
    direction: 'with-me', people: [{ name: r.owner, role: 'editor' }], hasLink: false,
  }))
  const byMe: ShareEntry[] = BY_ME_PEOPLE.map((r) => ({
    item: item(r.id, r.name, { isFolder: r.isFolder, size: r.size ?? null, childCount: r.items }),
    direction: 'by-me', people: r.people.map((name) => ({ name, role: 'viewer' as const })), hasLink: false,
  }))
  const links: ShareEntry[] = BY_ME_LINKS.map((r) => ({
    item: item(r.id, r.name, { size: r.size ?? null, shared: { kind: 'link' } }), direction: 'by-me', people: [], hasLink: true,
  }))
  return [...withMe, ...byMe, ...links]
}

export function createSharedSource(o: { empty?: boolean } = {}): DriveSource {
  const base = createFilesSource()
  let trash = o.empty ? [] : [...TRASH]
  return {
    ...base,
    shared: async () => (o.empty ? [] : shareEntries()),
    search: async (q) => SEARCH_ITEMS.filter((r) => r.item.name.toLowerCase().includes(q.trim().toLowerCase())).map((r) => r.item),
    trash: async () => trash,
    restoreFromTrash: async (ids) => { trash = trash.filter((t) => !ids.includes(t.item.id)) },
    deletePermanently: async (ids) => { trash = trash.filter((t) => !ids.includes(t.item.id)) },
  }
}
