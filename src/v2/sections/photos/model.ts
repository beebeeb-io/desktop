// Photos' pure model: days, totals, the "photos across" choice. No I/O besides the remembered choice (this PC only).
import type { PhotoEntry } from '@beebeeb/shared/drive-ui'
import type { PhotoMeta } from './types'

export const ACROSS = [3, 4, 5, 7] as const
export type Across = (typeof ACROSS)[number]
export const ACROSS_LABEL: Record<Across, string> = { 3: '3, larger', 4: '4', 5: '5', 7: '7, smallest' }
const KEY = 'bb.v2.photos.across'

export function loadAcross(): Across {
  try {
    const n = Number(window.localStorage.getItem(KEY))
    return (ACROSS as readonly number[]).includes(n) ? (n as Across) : 7
  } catch { return 7 }
}
export function saveAcross(n: Across): void {
  try { window.localStorage.setItem(KEY, String(n)) } catch { /* the choice just is not remembered */ }
}

/** What a tile needs: the entry plus whatever the source knows beyond PhotoEntry (device, length, uploading). */
export interface PhotoCell { entry: PhotoEntry; meta?: PhotoMeta }
export const idOf = (c: PhotoCell) => c.entry.item.id
export const isVideo = (c: PhotoCell) => (c.entry.item.mime ?? '').startsWith('video/')

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const two = (n: number) => String(n).padStart(2, '0')
export const dayKey = (iso: string) => iso.slice(0, 10)

/** "Today", "Yesterday", "Sunday, 12 October" (+ " · Lisbon" when the day has a place). */
export function dayHeading(key: string, now: Date, place?: string): string {
  const d = new Date(`${key}T12:00:00`)
  const days = Math.round((new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() - new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()) / 86_400_000)
  const base = days === 0 ? 'Today' : days === 1 ? 'Yesterday' : `${WEEKDAYS[d.getDay()]}, ${d.getDate()} ${MONTHS[d.getMonth()]}${d.getFullYear() < now.getFullYear() - 1 ? ` ${d.getFullYear()}` : ''}`
  return place ? `${base} · ${place}` : base
}

export interface Day { key: string; cells: PhotoCell[] }
/** Newest first, grouped by the day taken. */
export function groupByDay(cells: PhotoCell[]): Day[] {
  const sorted = [...cells].sort((a, b) => b.entry.takenAt.localeCompare(a.entry.takenAt))
  const days: Day[] = []
  for (const c of sorted) {
    const key = dayKey(c.entry.takenAt)
    const last = days[days.length - 1]
    if (last?.key === key) last.cells.push(c)
    else days.push({ key, cells: [c] })
  }
  return days
}

export function fmtMB(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`
}
export const fmtN = (n: number) => n.toLocaleString('en-US')

export function takenLabel(c: PhotoCell, now: Date): string {
  const d = new Date(c.entry.takenAt)
  const key = dayKey(c.entry.takenAt)
  const head = dayHeading(key, now).split(',')[0]
  const when = head === 'Today' || head === 'Yesterday' ? head : `${WEEKDAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()].slice(0, 3)}`
  return `${when} ${two(d.getHours())}:${two(d.getMinutes())}${c.meta?.device ? ` · ${c.meta.device}` : ''}`
}

export function fmtExt(name: string): string { return (name.split('.').pop() ?? '').toUpperCase() }

export const mb1 = (bytes: number) => `${(bytes / 1e6).toFixed(1)} MB`

/** B02b: the engine's metadata (read from the copy on this PC) as a cell's meta; only what it found. */
export function cellMeta(m: { camera?: string; width?: number; height?: number; durationSeconds?: number; uploading: boolean; device?: string } | undefined): PhotoCell['meta'] {
  if (!m) return undefined
  const d = m.durationSeconds
  return {
    camera: m.camera, uploading: m.uploading || undefined, device: m.device,
    dims: m.width && m.height ? `${m.width} × ${m.height}` : undefined,
    duration: d !== undefined ? `${Math.floor(d / 60)}:${String(d % 60).padStart(2, '0')}` : undefined,
  }
}

/** The devices the photos came from, most photos first (the pills, DW-0601), with how many each. */
export function devicesOf(cells: PhotoCell[]): { name: string; photos: number }[] {
  const counts = new Map<string, number>()
  for (const c of cells) if (c.meta?.device) counts.set(c.meta.device, (counts.get(c.meta.device) ?? 0) + 1)
  return [...counts].map(([name, photos]) => ({ name, photos })).sort((a, b) => b.photos - a.photos || a.name.localeCompare(b.name))
}

/** What Photos shows (DW-0604): every cell but those of a hidden device or a hidden folder. */
/** A folder source's key in the hidden set. Photos directly in Files (drive_photo_sources answers a null id) have one
 *  too, so switching that source off hides them (desktop #273 finding 6). */
export const folderKey = (id: string | null | undefined) => `folder:${id ?? ROOT_KEY}`
const ROOT_KEY = '(files)'

export function shown(cells: PhotoCell[], hidden: ReadonlySet<string>): PhotoCell[] {
  if (hidden.size === 0) return cells
  return cells.filter((c) => !(c.meta?.device && hidden.has(`device:${c.meta.device}`)) && !hidden.has(folderKey(c.entry.item.parentId)))
}

const HIDDEN_KEY = 'bb.v2.photos.hidden'
export function loadHidden(): Set<string> {
  try {
    const v = JSON.parse(window.localStorage.getItem(HIDDEN_KEY) ?? '[]')
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])
  } catch { return new Set() }
}
export function saveHidden(hidden: ReadonlySet<string>): void {
  try { window.localStorage.setItem(HIDDEN_KEY, JSON.stringify([...hidden])) } catch { /* the choice just is not remembered */ }
}

export function totalsOf(cells: PhotoCell[]): { photos: number; videos: number; bytes: number } {
  return { photos: cells.filter((c) => !isVideo(c)).length, videos: cells.filter(isVideo).length, bytes: cells.reduce((n, c) => n + (c.entry.item.size ?? 0), 0) }
}

/** Whether the window can read a video to play it (DW-0611), decided before any byte is read: over Rust's read cap it
 *  is too large, and a video not on this PC needs its size free to be decrypted here (#273 finding 7). */
export function videoReadable(item: { size: number | null; local?: string | null }, free: number | undefined, cap: number): 'ok' | 'too-large' | 'no-room' {
  const size = item.size ?? 0
  if (size > cap) return 'too-large'
  if (item.local === 'online-only' && free !== undefined && size > free) return 'no-room'
  return 'ok'
}
