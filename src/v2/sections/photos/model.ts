// Photos' pure model: days, totals, the "photos across" choice. No I/O besides the remembered choice (this PC only).
import type { PhotoEntry } from '@beebeeb/shared/drive-ui'
import type { PhotoMeta } from '../../source/fixtures/photos'

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
export function cellMeta(m: { camera?: string; width?: number; height?: number; durationSeconds?: number; uploading: boolean } | undefined): PhotoCell['meta'] {
  if (!m) return undefined
  const d = m.durationSeconds
  return {
    camera: m.camera, uploading: m.uploading || undefined,
    dims: m.width && m.height ? `${m.width} × ${m.height}` : undefined,
    duration: d !== undefined ? `${Math.floor(d / 60)}:${String(d % 60).padStart(2, '0')}` : undefined,
  }
}
