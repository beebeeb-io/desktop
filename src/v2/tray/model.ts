// The flyout's state model (B09): which of the board's states holds, and the words for it.
// The precedence is the desktop's own (24 Oct spec 6.3, POPOVER_PHASES), read from the snapshot Rust computes:
// this file keeps the order of the states the snapshot cannot carry (update required, checking, no plan) and
// nothing else. Pure: no I/O, so every state is a unit test.

import type { ActivityRow, PopoverSnapshot } from '../../popoverContract'

export type FlyoutKind =
  | 'update-required'
  | 'locked'
  | 'checking'
  | 'no-plan'
  | 'signed-out'
  | 'finder-failed'
  | 'finder-adding'
  | 'paused'
  | 'storage-full'
  | 'offline'
  | 'stopped'
  | 'needs-decision'
  | 'update-available'
  | 'metered'
  | 'storage-almost-full'
  | 'syncing'
  | 'up-to-date'

/** What the snapshot cannot say by itself. Every field is optional: absent means "not the case". */
export interface FlyoutExtras {
  updateRequired?: boolean
  /** Before the first read of the engine's state: a skeleton, no words. */
  checking?: boolean
  noPlan?: boolean
  /** Sync waits on a metered connection with "Sync on metered connections" off. */
  metered?: boolean
  /** Files Beebeeb stopped retrying. */
  stoppedFiles?: number
  /** Total files in Beebeeb, for "1,284 files". */
  totalFiles?: number
  /** Bytes per second of the upload now, for the mono figure beside the bar. */
  uploadBps?: number | null
  /** Seconds the batch will take, for "about a minute". */
  etaSeconds?: number | null
  /** The error Windows returned for a failed Cloud Files registration, in mono. */
  finderCode?: string | null
}

export type FlyoutCommand =
  | 'pause'
  | 'resume'
  | 'review'
  | 'try-again'
  | 'show-failed'
  | 'add-storage'
  | 'download-update'
  | 'unlock'
  | 'choose-plan'
  | 'sign-in'
  | 'finder-try-again'
  | 'sync-anyway'

export interface FlyoutAction {
  label: string
  command: FlyoutCommand
  /** Amber: only when it is the state's main action. */
  primary: boolean
}

export type FlyoutIcon = 'check' | 'sync' | 'pause' | 'alert' | 'lock' | 'cloud-off' | 'db' | 'download' | 'user' | 'folder' | 'plan' | 'wait'

export interface FlyoutRow {
  id: string
  name: string
  /** "Uploaded to Contracts", "Downloaded from Anna's MacBook": what happened and where. */
  meta: string
  /** now, 2 min, 44 min, 2 h, Yesterday, a weekday, 26 Oct. */
  time: string
  direction: 'up' | 'down' | null
  /** 0..100 while moving; null when no bar. */
  percent: number | null
  failed: boolean
  /** The line in red on a failed row. */
  failedLine: string | null
}

export type ListMode =
  | { kind: 'rows'; rows: FlyoutRow[] }
  | { kind: 'hidden'; title: string; line: string }
  | { kind: 'empty'; title: string }

export interface FlyoutView {
  kind: FlyoutKind
  tone: 'good' | 'busy' | 'attention' | 'neutral' | 'error' | 'locked'
  icon: FlyoutIcon | null
  /** Null for the skeleton: no words. */
  title: string | null
  line: string | null
  /** The mono reason (the Windows error code), when the state has one. */
  reason: string | null
  action: FlyoutAction | null
  /** Syncing and adding: the bar. Null percent = moving, amount unknown. */
  progress: { percent: number | null; speed: string | null } | null
  list: ListMode
  /** Open in File Explorer is disabled only while Beebeeb is not in File Explorer. */
  explorerEnabled: boolean
}

export const RECENT_LIMIT = 50
export const NO_RECENT_CHANGES = 'No changes yet'
export const LIST_LABEL = 'Recent changes'
export const LIST_COUNT = 'last 50'
export const LIST_END = "That's the last 50 changes."
export const VIEW_FULL_HISTORY = 'View full history'

export function formatCount(n: number): string {
  return n.toLocaleString('en-US')
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

/** Bytes as the board writes them: 48 MB, 912 GB, 1 TB. */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let i = 0
  while (value >= 1000 && i < units.length - 1) {
    value /= 1000
    i += 1
  }
  const text = value >= 100 || i === 0 ? Math.round(value).toString() : String(Math.round(value * 10) / 10)
  return `${text} ${units[i]}`
}

/** "about a minute", rounded, never a second-by-second countdown. */
export function etaWords(seconds: number): string {
  if (seconds < 45) return 'under a minute'
  if (seconds < 90) return 'about a minute'
  if (seconds < 55 * 60) return `${Math.max(2, Math.round(seconds / 300) * 5)} min`
  return 'about an hour'
}

export function speedWords(bps: number): string {
  return `${formatBytes(bps)}/s`
}

const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** now, 2 min, 44 min, 2 h, Yesterday, a weekday within 7 days, then 26 Oct. */
export function rowTime(at: number, nowSeconds: number): string {
  const sec = Math.max(0, nowSeconds - at)
  if (sec < 45) return 'now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min`
  const hours = Math.round(min / 60)
  if (hours < 24 && new Date(at * 1000).getDate() === new Date(nowSeconds * 1000).getDate()) return `${hours} h`
  const day = (s: number) => Math.floor(s / 86400)
  const days = day(nowSeconds - new Date().getTimezoneOffset() * 60) - day(at - new Date().getTimezoneOffset() * 60)
  if (days <= 1) return 'Yesterday'
  const when = new Date(at * 1000)
  if (days < 7) return WEEKDAYS[when.getDay()]
  return `${when.getDate()} ${MONTHS[when.getMonth()]}`
}

export function rowFromActivity(row: ActivityRow, nowSeconds: number): FlyoutRow {
  const where = row.folder ? row.folder : 'Beebeeb'
  const failed = row.state === 'failed'
  const moving = row.state === 'active'
  const percent = moving && row.done_bytes !== null && row.total_bytes ? Math.min(100, Math.round((row.done_bytes / row.total_bytes) * 100)) : null
  return {
    id: row.id,
    name: row.name,
    meta: failed
      ? where
      : row.direction === 'down'
        ? `${moving ? 'Downloading to' : 'Downloaded to'} ${where}`
        : `${moving ? 'Uploading to' : 'Uploaded to'} ${where}`,
    time: moving ? 'now' : rowTime(row.at, nowSeconds),
    direction: row.direction,
    percent: moving ? (percent ?? 0) : null,
    failed,
    failedLine: failed ? 'Couldn’t upload · open in another app' : null,
  }
}

const FINDER_REASON_LINE = 'Windows didn’t finish adding Beebeeb to File Explorer.'

function lastChange(snapshot: PopoverSnapshot, nowSeconds: number): string {
  const newest = snapshot.activity.find((r) => r.state === 'done')
  const when = newest ? rowTime(newest.at, nowSeconds) : null
  return when ? `last change ${when === 'now' ? 'just now' : /^\d+ (min|h)$/.test(when) ? `${when} ago` : when}` : ''
}

/**
 * The flyout's state. The first state that holds wins, in the board's order: update required, locked, checking,
 * no plan, signed out, File Explorer failed, File Explorer adding, paused, storage full, offline, stopped; then
 * needs a decision and update available over the quieter states; then metered, storage almost full, syncing,
 * up to date.
 */
export function flyoutView(snapshot: PopoverSnapshot, extras: FlyoutExtras = {}, nowSeconds: number = snapshot.generated_at): FlyoutView {
  const rows = snapshot.activity.slice(0, RECENT_LIMIT).map((r) => rowFromActivity(r, nowSeconds))
  const explorerEnabled = snapshot.finder.setup === 'ready'
  const list: ListMode = rows.length > 0 ? { kind: 'rows', rows } : { kind: 'empty', title: NO_RECENT_CHANGES }
  const base = { reason: null, action: null, progress: null, list, explorerEnabled } as const
  const act = (label: string, command: FlyoutCommand, primary = true): FlyoutAction => ({ label, command, primary })

  if (extras.updateRequired) {
    return { ...base, kind: 'update-required', tone: 'attention', icon: 'download', title: 'Update Beebeeb', line: 'This version of Beebeeb is too old to connect. Update to continue.', action: act('Download update', 'download-update') }
  }
  if (snapshot.phase === 'locked') {
    return {
      ...base, kind: 'locked', tone: 'locked', icon: 'lock', title: 'Beebeeb is locked', line: 'Sync keeps running while it’s locked.',
      action: act('Unlock with Windows Hello', 'unlock'),
      list: { kind: 'hidden', title: 'Recent changes are hidden', line: 'File names stay hidden while Beebeeb is locked.' },
    }
  }
  if (extras.checking) {
    return { ...base, kind: 'checking', tone: 'neutral', icon: null, title: null, line: null }
  }
  if (extras.noPlan) {
    return {
      ...base, kind: 'no-plan', tone: 'neutral', icon: 'plan', title: 'Choose a plan',
      line: 'Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.',
      action: act('Choose a plan on beebeeb.io', 'choose-plan'), list: { kind: 'empty', title: NO_RECENT_CHANGES },
    }
  }
  if (snapshot.phase === 'session_ended' || snapshot.phase === 'signed_out') {
    return { ...base, kind: 'signed-out', tone: 'attention', icon: 'user', title: 'Signed out', line: 'Enter your password to keep syncing. Nothing has been lost.', action: act('Sign in', 'sign-in') }
  }
  if (snapshot.phase === 'finder_failed' || snapshot.phase === 'finder_user_disabled') {
    return {
      ...base, kind: 'finder-failed', tone: 'error', icon: 'alert', title: 'Couldn’t add Beebeeb to File Explorer', line: FINDER_REASON_LINE,
      reason: extras.finderCode ?? snapshot.finder.reason, action: act('Try again', 'finder-try-again'),
    }
  }
  if (snapshot.phase === 'finder_adding' || snapshot.phase === 'finder_missing') {
    return {
      ...base, kind: 'finder-adding', tone: 'busy', icon: 'folder', title: 'Adding Beebeeb to File Explorer…',
      line: 'Beebeeb adds itself to File Explorer after sign-in.', progress: { percent: null, speed: null }, list: { kind: 'empty', title: NO_RECENT_CHANGES },
    }
  }
  if (snapshot.phase === 'paused') {
    const n = snapshot.pending_changes
    return { ...base, kind: 'paused', tone: 'neutral', icon: 'pause', title: 'Sync is paused', line: `${formatCount(n)} ${plural(n, 'change waits', 'changes wait')} on this PC`, action: act('Resume', 'resume') }
  }
  if (snapshot.phase === 'storage_full' || snapshot.storage_full) {
    const n = snapshot.pending_changes
    return {
      ...base, kind: 'storage-full', tone: 'error', icon: 'db', title: 'Your storage is full',
      line: `${formatCount(n)} ${plural(n, 'change can’t', 'changes can’t')} upload. They stay on this PC.`, action: act('Add storage', 'add-storage'),
    }
  }
  if (snapshot.phase === 'offline') {
    return { ...base, kind: 'offline', tone: 'neutral', icon: 'cloud-off', title: 'Can’t reach Beebeeb', line: 'Files on this PC are unchanged', action: act('Try again', 'try-again', false) }
  }
  if ((extras.stoppedFiles ?? 0) > 0 || snapshot.phase === 'error') {
    const n = extras.stoppedFiles ?? 0
    const failedFirst = [...rows].sort((a, b) => Number(b.failed) - Number(a.failed))
    return {
      ...base, kind: 'stopped', tone: 'error', icon: 'alert', title: 'Sync stopped',
      line: n > 0 ? `${formatCount(n)} ${plural(n, 'file couldn’t', 'files couldn’t')} upload. Nothing was lost.` : 'Something stopped the sync. Nothing was lost.',
      action: act('Show', 'show-failed'), list: rows.length > 0 ? { kind: 'rows', rows: failedFirst } : list,
    }
  }
  if (snapshot.conflicts.count > 0) {
    const n = snapshot.conflicts.count
    return { ...base, kind: 'needs-decision', tone: 'attention', icon: 'alert', title: `${formatCount(n)} ${plural(n, 'file needs', 'files need')} a decision`, line: 'Changed in two places', action: act('Review…', 'review') }
  }
  if (snapshot.update) {
    return { ...base, kind: 'update-available', tone: 'attention', icon: 'download', title: `Version ${snapshot.update.version} is available`, line: 'Download it, then open it to install.', action: act('Download update', 'download-update', false) }
  }
  if (extras.metered) {
    return { ...base, kind: 'metered', tone: 'neutral', icon: 'wait', title: 'Sync is waiting', line: 'This PC is on a metered connection', action: act('Sync anyway', 'sync-anyway', false) }
  }
  const quota = snapshot.storage
  if (quota && quota.quota_bytes > 0 && quota.used_bytes / quota.quota_bytes >= 0.9) {
    return {
      ...base, kind: 'storage-almost-full', tone: 'attention', icon: 'db', title: 'Storage almost full',
      line: `${formatBytes(quota.used_bytes)} of ${formatBytes(quota.quota_bytes)}`, action: act('Add storage', 'add-storage', false),
    }
  }
  if (snapshot.sync_phase === 'syncing') {
    const left = snapshot.engine.files_remaining ?? rows.filter((r) => r.percent !== null).length
    const total = snapshot.engine.bytes_total
    const done = snapshot.engine.bytes_done ?? 0
    const leftBytes = total !== null ? Math.max(0, total - done) : null
    const eta = extras.etaSeconds ?? null
    const line = [leftBytes !== null ? `${formatBytes(leftBytes)} left` : null, eta !== null ? etaWords(eta) : null].filter(Boolean).join(' · ')
    const moving = rows.filter((r) => r.percent !== null)
    return {
      ...base, kind: 'syncing', tone: 'busy', icon: 'sync', title: `Syncing ${formatCount(left)} ${plural(left, 'file', 'files')}`, line: line || 'Working…',
      action: act('Pause', 'pause', false),
      progress: { percent: total ? Math.min(100, Math.round((done / total) * 100)) : null, speed: extras.uploadBps ? speedWords(extras.uploadBps) : null },
      list: rows.length > 0 ? { kind: 'rows', rows: [...moving, ...rows.filter((r) => r.percent === null)] } : list,
    }
  }
  const files = extras.totalFiles !== undefined ? `${formatCount(extras.totalFiles)} ${plural(extras.totalFiles, 'file', 'files')}` : ''
  const when = lastChange(snapshot, nowSeconds)
  return { ...base, kind: 'up-to-date', tone: 'good', icon: 'check', title: 'Up to date', line: [files, when].filter(Boolean).join(' · ') || null }
}
