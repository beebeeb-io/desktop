// Display formatting for the shared Drive UI. Core is the canonical byte formatter (web calls it through
// WASM, desktop through Rust); this module owns only the registry those hosts plug into, a plain
// fallback with the same ladder, and the date helpers that need no core.

export interface SizeFormatter {
  /** Decimal storage form (1 KB = 1,000 B). */
  si(bytes: number): string
  /** Decimal, at most one decimal, whole numbers bare ("2 GB"). */
  compact(bytes: number): string
  /** Binary/IEC ladder (1 KiB = 1,024 B), for preview surfaces. */
  binary?(bytes: number): string
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']

function ladder(bytes: number, decimals: number): string {
  let value = bytes
  let i = 0
  while (value >= 1000 && i < UNITS.length - 1) {
    value /= 1000
    i++
  }
  const fixed = i === 0 ? String(Math.round(value)) : value.toFixed(decimals)
  return `${fixed} ${UNITS[i]}`
}

const fallback: SizeFormatter = {
  si: (b) => ladder(b, 1),
  compact: (b) => ladder(b, 1).replace(/\.0 (B|KB|MB|GB|TB|PB)$/, ' $1'),
}

let active: SizeFormatter = fallback

/** A host swaps in core's formatter (web: WASM; desktop: the Rust command). */
export function registerSizeFormatter(formatter: SizeFormatter | null): void {
  active = formatter ?? fallback
}

/** Decimal storage form (1 KB = 1,000 B). */
export function formatStorageSI(bytes: number): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '—'
  return active.si(Math.round(bytes))
}

export function formatBytes(bytes: number): string {
  return formatStorageSI(bytes)
}

/** Binary/IEC form (1 KiB = 1,024 B). */
export function formatBinarySize(bytes: number): string {
  if (bytes == null || !Number.isFinite(bytes)) return '—'
  if (active.binary) return active.binary(Math.round(bytes))
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let v = bytes
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`
}

/** One decimal at most, a whole number shown bare ("2 GB", "6.3 GB"). */
export function formatSize(bytes: number): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '0 B'
  return active.compact(Math.round(bytes))
}

/** A document date ("12 Oct 2026"); a date-only value names a calendar day and formats in UTC. */
export function formatDocumentDate(iso: string): string {
  if (!iso) return '-'
  const opts: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', year: 'numeric' }
  const dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso)
  if (dateOnly) {
    const d = new Date(Date.UTC(+dateOnly[1], +dateOnly[2] - 1, +dateOnly[3]))
    if (Number.isNaN(d.getTime())) return iso
    return d.toLocaleDateString('en-GB', { ...opts, timeZone: 'UTC' })
  }
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return d.toLocaleDateString('en-GB', opts)
}

/** Relative age as Files shows it ("just now", "3h ago", "2w ago"). */
export function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime()
  const seconds = Math.floor(diff / 1000)
  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  const weeks = Math.floor(days / 7)
  return `${weeks}w ago`
}
