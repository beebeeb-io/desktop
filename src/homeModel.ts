/**
 * Home's three additions as pure decisions (task 1683 slice 4, the Home add-on; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6 Home, §10.2 rows 3, 34 and 45; hi-fi section 3 "Home"): the
 * counts line inside the status card, which Review entries "Needs your attention" shows, and the "Recent activity" rows.
 * `HomeSections.tsx` draws them, `MacApp.tsx` `HomePane` gathers their inputs. Nothing here touches React or Tauri.
 */
import type { FileOverview, VersionConflictEntry } from './desktopApi'
import type { ActivityRow } from './popoverContract'
import type { ReviewFeedState } from './reviewFeed'
import { dayLabel, parseWhen } from './whenFormat'

/** How many activity rows the window asks the snapshot for (the popover shows five; Home shows "more of them"). */
export const HOME_ACTIVITY_ROWS = 8
/** "Needs your attention" shows the top three Review entries (§7.6). */
export const HOME_ATTENTION_LIMIT = 3

// ── The counts line ──────────────────────────────────────────────────────────

export interface HomeCountPart {
  label: string
  value: string
}

function numberIn(locale: string, value: number): string {
  try {
    return new Intl.NumberFormat(locale).format(value)
  } catch {
    // An invalid locale tag throws RangeError: English rather than no number.
    return new Intl.NumberFormat('en').format(value)
  }
}

/**
 * "On this Mac {n} files · Online-only {n} · Kept on this Mac {n} folders" as label and value pairs. "On this Mac" is the
 * `local` bucket and "Online-only" the `cloud_only` one, as the Windows shell derives its two (§10.2 row 34 only renames
 * the label); a status with no files is zero. The kept-folder part is left out when the overview does not carry it (a
 * build before the field), so an older answer cannot say "0 folders" by accident. No files mirrored at all (no sync folder,
 * no database, an empty vault; §14 item 2) is no line, never "0 files".
 */
export function homeCounts(overview: FileOverview | null, locale: string): HomeCountPart[] | null {
  if (overview === null || !(overview.total_files > 0)) return null
  const bucket = (status: string) => overview.by_status.find((b) => b.status === status)?.count ?? 0
  const local = bucket('local')
  const parts: HomeCountPart[] = [
    { label: 'On this Mac', value: `${numberIn(locale, local)} ${local === 1 ? 'file' : 'files'}` },
    { label: 'Online-only', value: numberIn(locale, bucket('cloud_only')) },
  ]
  if (typeof overview.pinned_folders === 'number') {
    const folders = overview.pinned_folders
    parts.push({ label: 'Kept on this Mac', value: `${numberIn(locale, folders)} ${folders === 1 ? 'folder' : 'folders'}` })
  }
  return parts
}

// ── Needs your attention ─────────────────────────────────────────────────────

/** The first three Review entries, and how many more wait in Review. A feed that has not answered shows none. */
export function attentionForHome(feed: ReviewFeedState): { entries: VersionConflictEntry[]; more: number } {
  if (feed.status !== 'ready') return { entries: [], more: 0 }
  return { entries: feed.entries.slice(0, HOME_ATTENTION_LIMIT), more: Math.max(0, feed.entries.length - HOME_ATTENTION_LIMIT) }
}

// ── Recent activity ──────────────────────────────────────────────────────────

/** "just now", "2 minutes ago", "3 hours ago", "yesterday", "12 Oct"; null when the time is unknown. */
export function relativeAgo(at: number, now: Date): string | null {
  const date = parseWhen(at)
  if (date === null) return null
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000)
  if (seconds < 60) return 'just now' // a clock that is a little ahead is "just now" too
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} ${minutes === 1 ? 'minute' : 'minutes'} ago`
  const day = dayLabel(date, now)
  if (day !== 'today') return day
  const hours = Math.floor(minutes / 60)
  return `${hours} ${hours === 1 ? 'hour' : 'hours'} ago`
}

export interface HomeActivityRow {
  id: string
  name: string
  /** The glyph: the direction, or an alert for a failure. */
  icon: 'up' | 'down' | 'alert'
  /** "Uploaded · Investigations · 2 minutes ago". */
  line: string
  /** The whole row as one sentence for VoiceOver, as the popover's rows are read (§11). */
  label: string
  path: string
  tone: 'neutral' | 'busy' | 'alert'
}

function progressPercent(row: ActivityRow): number | null {
  if (row.state !== 'active') return null
  const done = row.done_bytes
  const total = row.total_bytes
  if (done === null || total === null || !(total > 0)) return null // unknown is no percentage, never a guess
  return Math.min(100, Math.max(0, Math.round((done / total) * 100)))
}

/** The popover snapshot's rows, worded for Home (§7.6: "the popover's rows, more of them"). */
export function activityRowsForHome(rows: ActivityRow[], now: Date): HomeActivityRow[] {
  return rows.map((row) => {
    const failed = row.state === 'failed' || row.direction === null
    const up = row.direction === 'up'
    const verb = failed ? 'Couldn’t sync' : row.state === 'active' ? (up ? 'Uploading' : 'Downloading') : up ? 'Uploaded' : 'Downloaded'
    const percent = failed ? null : progressPercent(row)
    const ago = relativeAgo(row.at, now)
    const folder = row.folder.trim() === '' ? null : row.folder
    const line = [verb, percent === null ? null : `${percent} %`, folder, ago].filter((part): part is string => part !== null).join(' · ')
    const label = [row.name, verb.toLowerCase(), percent === null ? null : `${percent} percent`, folder, ago]
      .filter((part): part is string => part !== null)
      .join(', ')
    return {
      id: row.id,
      name: row.name,
      icon: failed ? 'alert' : up ? 'up' : 'down',
      line,
      label,
      path: row.path,
      tone: failed ? 'alert' : row.state === 'active' ? 'busy' : 'neutral',
    }
  })
}
