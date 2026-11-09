// Files section model: the board's words and pure helpers (DW-0401 to DW-0429). No data access here.
import { getFileType, type FileType } from '@beebeeb/shared/drive-ui'
import type { DriveItem, LocalState, SharedState, SortDir, SortKey } from '@beebeeb/shared/drive-ui'

export type ViewMode = 'list' | 'grid'
export interface SortPrefs { key: SortKey; dir: SortDir; foldersFirst: boolean }
export interface FolderPrefs { view: ViewMode; sort: SortPrefs; panel: boolean }
export const DEFAULT_PREFS: FolderPrefs = { view: 'list', sort: { key: 'name', dir: 'asc', foldersFirst: true }, panel: false }

/** The board's sort choices (DW-0404): the iPhone's names. */
export const SORT_LABEL: Record<SortKey, string> = { name: 'Name', modified: 'Date changed', size: 'Size', kind: 'Type' }
export function orderLabels(key: SortKey): [string, string] {
  if (key === 'modified') return ['Newest first', 'Oldest first']
  if (key === 'size') return ['Largest first', 'Smallest first']
  return ['A to Z', 'Z to A']
}

export const LOCAL_LABEL: Record<LocalState, string> = {
  'online-only': 'Online only',
  downloaded: 'On this PC',
  'available-offline': 'Available offline',
  syncing: 'Syncing',
  failed: 'Not synced',
}

export const BACKUPS_ID = 'f-backups'
/** Folders a service or a device backup manages: open and download only (DW-0425). */
export function isProtected(item: DriveItem): boolean {
  return item.readOnly === true || item.id === BACKUPS_ID
}

export function fmtSize(bytes: number | null): string {
  if (bytes == null) return '—'
  if (bytes < 1000) return `${bytes} B`
  if (bytes < 1e6) return `${Math.round(bytes / 1e3)} KB`
  if (bytes < 1e9) return `${(bytes / 1e6).toFixed(1)} MB`
  return `${(bytes / 1e9).toFixed(1)} GB`
}

export function fmtCount(n: number | undefined): string {
  return `${n ?? 0} ${n === 1 ? 'item' : 'items'}`
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const two = (n: number) => String(n).padStart(2, '0')

/** "Just now", "Today, 09:12", "Yesterday, 16:42", "28 Oct 2026". */
export function fmtWhen(iso: string, now: Date = new Date()): string {
  const d = new Date(iso)
  const diff = now.getTime() - d.getTime()
  if (diff < 60_000 && diff > -60_000) return 'Just now'
  // Calendar days, not 24-hour spans: a DST change between the two midnights would shift a local-midnight count.
  const day = (x: Date) => Date.UTC(x.getFullYear(), x.getMonth(), x.getDate()) / 86_400_000
  const hm = `${two(d.getHours())}:${two(d.getMinutes())}`
  if (day(now) === day(d)) return `Today, ${hm}`
  if (day(now) - day(d) === 1) return `Yesterday, ${hm}`
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`
}

export function fmtDay(iso: string): string {
  const d = new Date(iso)
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`
}

export function fmtShortWhen(iso: string): string {
  const d = new Date(iso)
  return `${d.getDate()} ${MONTHS[d.getMonth()]}, ${two(d.getHours())}:${two(d.getMinutes())}`
}

const KIND: Record<FileType, string> = {
  folder: 'Folder', pdf: 'PDF document', word: 'Word document', excel: 'Spreadsheet', powerpoint: 'Presentation',
  image: 'Image', video: 'Video', audio: 'Audio', code: 'Code', archive: 'Archive', data: 'Data file',
  md: 'Markdown document', fig: 'Design file', default: 'File',
}

export function kindOf(item: DriveItem): string {
  const type = getFileType(item.name, item.isFolder)
  const ext = item.name.includes('.') ? item.name.slice(item.name.lastIndexOf('.') + 1).toUpperCase() : ''
  if (type === 'image' && ext) return `${ext} image`
  if (type === 'video' && ext) return `${ext} video`
  return type === 'default' && ext ? `${ext} file` : KIND[type]
}

export function sharedLabel(s: SharedState): string {
  return s.kind === 'link' ? 'Link' : s.kind === 'people' ? s.people.map((p) => p.name).join(', ') : '—'
}

/**
 * The name a row shows. The engine's blocked label (`Unavailable file` plus 16 hex digits) is not a name:
 * the row keeps the short id so two such files stay distinct.
 */
export function rowName(name: string): string {
  const blocked = /^Unavailable file ([0-9a-f]{16})$/i.exec(name.trim())
  return blocked ? `File ${blocked[1]}` : name
}

/** Shortens a long name in the middle so the start, the number and the extension stay (DW-0420b). */
export function middleShorten(name: string, max = 46): string {
  if (name.length <= max) return name
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 && name.length - dot <= 6 ? name.slice(dot) : ''
  const stem = name.slice(0, name.length - ext.length)
  const tail = Math.min(10, Math.floor((max - ext.length) / 3))
  const head = max - ext.length - tail - 1
  return `${stem.slice(0, head)}…${stem.slice(stem.length - tail)}${ext}`
}

/** The name without its extension, for the rename selection (DW-0410). */
export function stemLength(item: Pick<DriveItem, 'name' | 'isFolder'>): number {
  const dot = item.name.lastIndexOf('.')
  return item.isFolder || dot <= 0 ? item.name.length : dot
}

/** The rules of DW-0410: no "/" or ":", not empty, at most 255 bytes. Returns the board's words or null. */
export function nameProblem(name: string): string | null {
  const t = name.trim()
  if (!t) return 'A name can’t be empty.'
  if (/[/:]/.test(t)) return 'A name can’t contain / or :.'
  if (new TextEncoder().encode(t).length > 255) return 'That name is too long. Use 255 bytes or fewer.'
  return null
}

export function nameTaken(name: string, siblings: readonly DriveItem[], ignoreId?: string): DriveItem | undefined {
  const t = name.trim().toLowerCase()
  return siblings.find((i) => i.id !== ignoreId && i.name.toLowerCase() === t)
}

export function takenMessage(name: string, isFolder: boolean): string {
  return `There’s already a ${isFolder ? 'folder' : 'file'} called ${name} here. Choose another name.`
}

/** Re-sorts a listing as the folder's preferences say (the source sorts by key and dir; folders-first is the view's). */
export function arrange(items: DriveItem[], prefs: SortPrefs): DriveItem[] {
  const sign = prefs.dir === 'asc' ? 1 : -1
  const by = (a: DriveItem, b: DriveItem): number => {
    if (prefs.key === 'modified') return a.modifiedAt.localeCompare(b.modifiedAt)
    if (prefs.key === 'size') return (a.size ?? a.childCount ?? 0) - (b.size ?? b.childCount ?? 0)
    if (prefs.key === 'kind') return kindOf(a).localeCompare(kindOf(b)) || a.name.localeCompare(b.name)
    return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })
  }
  return [...items].sort((a, b) => {
    // Backups stays the first row whatever the sort (DW-0404).
    if (a.id === BACKUPS_ID) return -1
    if (b.id === BACKUPS_ID) return 1
    if (prefs.foldersFirst && a.isFolder !== b.isFolder) return a.isFolder ? -1 : 1
    return sign * by(a, b)
  })
}

const KEY = 'bb.v2.folder-prefs'
export function loadPrefs(folderId: string | null): FolderPrefs {
  try {
    const all = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, FolderPrefs>
    return { ...DEFAULT_PREFS, ...all[folderId ?? 'root'] }
  } catch { return DEFAULT_PREFS }
}
export function savePrefs(folderId: string | null, prefs: FolderPrefs): void {
  try {
    const all = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, FolderPrefs>
    all[folderId ?? 'root'] = prefs
    localStorage.setItem(KEY, JSON.stringify(all))
  } catch { /* a private window keeps the default */ }
}

/**
 * What to say when this version cannot run an action. Names the action in plain words and says what to do next.
 * Never a build word or an internal code.
 */
export function unavailableWords(action: string): string {
  const key = action.trim().toLowerCase()
  if (key === 'open' || key === 'open it') return 'Couldn’t open it. Open it from File Explorer.'
  if (key === 'reveal' || key === 'show it in file explorer') return 'Couldn’t show it in File Explorer. Try again.'
  if (key === 'keep offline' || key === 'keep it on this pc') return 'Couldn’t keep it on this PC. Try again.'
  return `Couldn’t ${action}. Not in this version.`
}

/**
 * A failed action's message (DW-0109): what failed and how to fix it, never a raw error code. `retry` says whether
 * Try again can help: not when this version lacks the action, the account is full, or the name is already used.
 */
export function actionFailure(action: string, code: string): { text: string; retry: boolean } {
  if (code === 'offline') return { text: `Couldn’t ${action}. Check your connection.`, retry: true }
  if (code === 'quota') return { text: `Couldn’t ${action}. Not enough space.`, retry: false }
  if (code === 'unsupported') return { text: unavailableWords(action), retry: false }
  if (code === 'signed-out') return { text: `Couldn’t ${action}. Sign in to continue.`, retry: false }
  if (code === 'name-taken') return { text: `Couldn’t ${action}. That name is already used here.`, retry: false }
  return { text: `Couldn’t ${action}.`, retry: true }
}

/** Type-to-select (DW-0420b): letters typed within this long of each other build one prefix, as File Explorer does. */
export const TYPEAHEAD_MS = 1000

/** The prefix after one more key, and the index of the first name that starts with it (or -1). */
export function typeahead(prev: { text: string; at: number } | null, key: string, at: number, names: string[]): { text: string; at: number; hit: number } {
  const text = prev && at - prev.at < TYPEAHEAD_MS ? prev.text + key : key
  const want = text.toLowerCase()
  return { text, at, hit: names.findIndex((n) => n.toLowerCase().startsWith(want)) }
}
