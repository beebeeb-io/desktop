/**
 * What sync is doing, in words: the one model Settings › Sync and the Windows tray popover render (0.8.15).
 *
 * On Guus's PC (0.8.14) the popover said "Your files are synced" while File Explorer showed files pending: 2,924
 * changes had been paused since 4 Oct, 43 had given up, and the engine was stuck in its startup check. Neither surface
 * read any of that. This model reads `sync_activity_summary` (`src-tauri/src/sync_activity.rs`) next to `sync_status`,
 * and never says "up to date" or "syncing" while the queue says otherwise.
 *
 * Copy rules: counts and dates only, never a file name; honest, never reassuring. The changes paused for `auth` stay
 * paused in 0.8.15 by the Mac lead's ruling (nothing here offers to resume them); a later update repairs them.
 */
import type { SyncStatus } from '../desktopApi'

export interface HeldGroup {
  /** `auth`, `quota`, `permission`, `locked`, `key_replaced`, or `other`. */
  reason: string
  count: number
  /** Unix seconds the oldest and newest of them were queued (0 = not recorded). */
  oldest_at: number
  newest_at: number
}

export interface QueueHold {
  /** Not paused and not given up: due now, backing off, or being sent. */
  waiting: number
  /** Out of attempts: nothing retries them. */
  gave_up: number
  held: HeldGroup[]
}

export interface StartupCheck {
  done: number
  total: number
}

/** `sync_activity_summary`, field for field (`SyncActivitySummary` in Rust). */
export interface SyncActivitySummary {
  paused: boolean
  engine_state: string
  files_remaining: number | null
  transfers_active: number
  last_tick_ok_at: number | null
  queue: QueueHold | null
  startup_check: StartupCheck | null
}

type Obj = Record<string, unknown>
const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

function isHeldGroup(v: unknown): v is HeldGroup {
  return isObj(v) && typeof v.reason === 'string' && isCount(v.count) && isNum(v.oldest_at) && isNum(v.newest_at)
}

/** Narrow a `sync_activity_summary` result. Null when it is not the contract's shape: the surfaces then fall back to `sync_status` alone. */
export function parseSyncActivitySummary(raw: unknown): SyncActivitySummary | null {
  if (!isObj(raw)) return null
  if (typeof raw.paused !== 'boolean' || typeof raw.engine_state !== 'string') return null
  if (!(raw.files_remaining === null || isCount(raw.files_remaining))) return null
  if (!isCount(raw.transfers_active)) return null
  if (!(raw.last_tick_ok_at === null || isNum(raw.last_tick_ok_at))) return null
  const { queue, startup_check: check } = raw
  if (queue !== null && !(isObj(queue) && isCount(queue.waiting) && isCount(queue.gave_up) && Array.isArray(queue.held) && queue.held.every(isHeldGroup))) return null
  if (check !== null && !(isObj(check) && isCount(check.done) && isCount(check.total))) return null
  return raw as unknown as SyncActivitySummary
}

/**
 * The start-up guard's pass while it runs. `sync_status.startup_guard` is read live on every poll, so when the status
 * carries it, it decides (a pass at rest is no check, whatever the summary last heard). Without it, the pass the last
 * `engine-status` event reported (`summary.startup_check`). A value outside the shape is no check, never a guess.
 */
export function startupCheckFrom(status: SyncStatus | null, summary: SyncActivitySummary | null | undefined): StartupCheck | null {
  const live: unknown = status?.startup_guard
  if (live !== undefined && live !== null) {
    if (!isObj(live) || live.running !== true || !isCount(live.done) || !isCount(live.total) || live.total === 0) return null
    return { done: Math.min(live.done, live.total), total: live.total }
  }
  const last = summary?.startup_check ?? null
  return last && last.total > 0 ? last : null
}

// ── Words ───────────────────────────────────────────────────────────────────

export type SyncActivityKind =
  | 'unknown'
  | 'signed_out'
  | 'session_ended'
  | 'refused'
  | 'paused'
  | 'checking'
  | 'conflicts'
  | 'syncing'
  | 'offline'
  | 'error'
  | 'starting'
  | 'waiting'
  | 'on_hold'
  | 'locked'
  | 'stopped'
  | 'reading'
  | 'unconfirmed'
  | 'up_to_date'

/** `good` = nothing pending; `busy` = work is moving; `attention` = something will not move by itself. */
export type SyncActivityTone = 'good' | 'busy' | 'attention' | 'neutral'

export interface SyncActivityView {
  kind: SyncActivityKind
  /** A word or two for a chip or the "State" figure. */
  label: string
  /** One sentence: what is going on. */
  headline: string
  /** Further sentences, counts only. */
  details: string[]
  /** The startup check's progress while it runs. */
  progress: StartupCheck | null
  tone: SyncActivityTone
  /** Signed in, so Pause / Resume applies. */
  pausable: boolean
  /** The live pause flag (or the caller's fallback when the summary is unavailable). */
  paused: boolean
}

export interface SyncActivityOptions {
  /** "this PC" on Windows, "this device" elsewhere. */
  device?: string
  /** The pause state to show when the summary could not be read (for example `desktop_config.pause_sync`). */
  pausedFallback?: boolean
  /** For tests: the date "now" is, which decides whether a year is shown. */
  now?: Date
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export function formatCount(n: number): string {
  return n.toLocaleString('en-US')
}

function plural(n: number, one: string, many: string): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`
}

/** "4 Oct", or "4 Oct 2025" outside the current year. Local time. */
export function shortDate(unixSeconds: number, now: Date = new Date()): string {
  const d = new Date(unixSeconds * 1000)
  const base = `${d.getDate()} ${MONTHS[d.getMonth()]}`
  return d.getFullYear() === now.getFullYear() ? base : `${base} ${d.getFullYear()}`
}

/** "from 4 Oct" when all of them were queued that day, "since 4 Oct" when they span days, "" when not recorded. */
function datePhrase(group: HeldGroup, now: Date): string {
  if (group.oldest_at <= 0) return ''
  const first = shortDate(group.oldest_at, now)
  const last = group.newest_at > 0 ? shortDate(group.newest_at, now) : first
  return first === last ? ` from ${first}` : ` since ${first}`
}

/** One sentence per pause reason. Only `auth` promises a repair: the Mac lead's ruling for 0.8.15. */
export function heldSentence(group: HeldGroup, now: Date = new Date()): string {
  const n = group.count
  const changes = plural(n, 'change', 'changes')
  const are = n === 1 ? 'is' : 'are'
  switch (group.reason) {
    case 'auth':
      return `${changes}${datePhrase(group, now)} ${are} on hold. An update will repair ${n === 1 ? 'it' : 'them'}.`
    case 'quota':
      return `${changes} ${are} on hold because your storage is full.`
    case 'permission':
      return `${changes} ${are} on hold because you don’t have permission to make ${n === 1 ? 'it' : 'them'}.`
    case 'locked':
      return `${changes} ${are} on hold while Beebeeb is locked.`
    case 'key_replaced':
      return `${changes} ${are} on hold: ${n === 1 ? 'it was' : 'they were'} made with a key this account no longer uses.`
    default:
      return `${changes}${datePhrase(group, now)} ${are} on hold.`
  }
}

export function gaveUpSentence(n: number): string {
  return `${plural(n, 'change', 'changes')} stopped after repeated errors and ${n === 1 ? 'is' : 'are'} not retried.`
}

export function waitingSentence(n: number): string {
  return `${plural(n, 'change', 'changes')} waiting to sync.`
}

/**
 * The view for one `sync_status` and one `sync_activity_summary` (either may be missing). Precedence, highest first:
 * not signed in, session ended, engine refused, paused, startup check, conflicts, moving, offline, error, starting,
 * waiting, on hold, locked, not running, up to date. Whatever is pending and not the headline is a detail line, so a
 * surface never shows a calm state over a queue that is not empty.
 *
 * `summary` is `undefined` before the first read and `null` when it could not be read. Neither ever ends in "up to
 * date": without the queue there is no way to know that.
 */
export function syncActivityView(
  status: SyncStatus | null,
  summary: SyncActivitySummary | null | undefined,
  options: SyncActivityOptions = {},
): SyncActivityView {
  const device = options.device ?? 'this PC'
  const now = options.now ?? new Date()
  const paused = summary?.paused ?? options.pausedFallback ?? false
  const base = { details: [] as string[], progress: null, pausable: status?.logged_in === true, paused }

  if (status == null && summary == null) {
    return summary === undefined
      ? { ...base, kind: 'reading', label: '…', headline: 'Reading what sync is doing…', tone: 'neutral' }
      : { ...base, kind: 'unknown', label: 'Unknown', headline: 'Sync status is unavailable.', tone: 'neutral' }
  }
  if (status != null && !status.logged_in) {
    return { ...base, kind: 'signed_out', label: 'Signed out', headline: 'Sign in to start syncing.', tone: 'neutral' }
  }
  if (status?.auth_expired) {
    return { ...base, kind: 'session_ended', label: 'Signed out', headline: `Signed out on ${device}. Sign in again to sync.`, tone: 'attention' }
  }

  const queue = summary?.queue ?? null
  const waiting = queue?.waiting ?? 0
  const held = (queue?.held ?? []).filter((group) => group.count > 0)
  const gaveUp = queue?.gave_up ?? 0
  const heldLines = held.map((group) => heldSentence(group, now))
  const gaveUpLines = gaveUp > 0 ? [gaveUpSentence(gaveUp)] : []
  const moving = summary
    ? summary.transfers_active > 0 || summary.engine_state === 'syncing'
    : (status?.syncing ?? 0) > 0
  const movingCount = summary ? (summary.files_remaining ?? summary.transfers_active) : (status?.syncing ?? 0)
  const engineState = summary?.engine_state ?? ''
  const conflicts = status?.conflicts ?? 0
  const conflictLine = conflicts > 0 ? `${plural(conflicts, 'conflict', 'conflicts')} to review.` : null
  const check = startupCheckFrom(status, summary)
  const checkLine = check ? `Checking files on ${device}: ${formatCount(check.done)} of ${formatCount(check.total)}.` : null

  const pick = (
    kind: SyncActivityKind,
    label: string,
    headline: string,
    tone: SyncActivityTone,
    extra: { waitingLine?: boolean; progress?: StartupCheck | null; lead?: string[] } = {},
  ): SyncActivityView => ({
    ...base,
    kind,
    label,
    headline,
    tone,
    progress: extra.progress ?? null,
    details: [
      ...(extra.lead ?? []),
      ...(kind !== 'conflicts' && conflictLine ? [conflictLine] : []),
      ...(extra.waitingLine !== false && waiting > 0 ? [waitingSentence(waiting)] : []),
      ...heldLines,
      ...gaveUpLines,
    ],
  })

  if (status?.engine_refusal) {
    return pick('refused', 'Not syncing', status.engine_refusal.sentence, 'attention')
  }
  if (paused) {
    return pick('paused', 'Paused', 'Sync is paused.', 'neutral', {
      waitingLine: false,
      progress: check,
      lead: [
        ...(waiting > 0 ? [`${plural(waiting, 'change waits', 'changes wait')} until you resume.`] : []),
        ...(checkLine ? [checkLine] : []),
      ],
    })
  }
  const movingHeadline = movingCount > 0 ? `${plural(movingCount, 'file', 'files')} syncing.` : 'Syncing.'
  if (check && checkLine) {
    // Since 0.8.15 uploads and downloads run during the pass (the engine lane's non-blocking guard): what moves is the
    // headline then, and the pass stays in view under it.
    return moving
      ? pick('syncing', 'Syncing', movingHeadline, 'busy', { progress: check, lead: [checkLine] })
      : pick('checking', 'Checking', checkLine, 'busy', { progress: check })
  }
  if (conflicts > 0 && conflictLine) {
    return pick('conflicts', 'Needs review', conflictLine, 'attention')
  }
  if (moving) {
    return pick('syncing', 'Syncing', movingHeadline, 'busy')
  }
  if (engineState === 'offline') {
    return pick('offline', 'Offline', 'Offline. Beebeeb tries again when the connection is back.', 'attention')
  }
  if (engineState === 'error') {
    return pick('error', 'Error', 'The last sync check failed.', 'attention')
  }
  if (engineState === 'running') {
    return pick('starting', 'Starting', 'Starting. No sync check has finished yet.', 'busy')
  }
  if (waiting > 0) {
    return pick('waiting', 'Waiting', waitingSentence(waiting), 'busy', { waitingLine: false })
  }
  if (heldLines.length > 0 || gaveUpLines.length > 0) {
    const [headline, ...rest] = [...heldLines, ...gaveUpLines]
    return { ...base, kind: 'on_hold', label: 'On hold', headline, details: rest, tone: 'attention' }
  }
  if (status?.engine === 'locked') {
    return pick('locked', 'Locked', 'Beebeeb is locked. Unlock it to sync.', 'neutral')
  }
  if (status != null && status.engine !== 'running') {
    return pick('stopped', 'Not running', 'Sync is not running.', 'neutral')
  }
  if (summary === undefined) {
    return pick('reading', '…', 'Reading what sync is doing…', 'neutral')
  }
  if (summary === null) {
    return pick('unconfirmed', 'Running', 'Sync is running. What is queued could not be read.', 'neutral')
  }
  return pick('up_to_date', 'Up to date', 'Your files are synced.', 'good')
}
