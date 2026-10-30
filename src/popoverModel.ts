/**
 * What the menu-bar popover shows (task 1683 slice 3; two-surface spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §6.3, §6.4, §10, §11). One snapshot in, one view out: the header,
 * the body, the status row, the one action, the footer and where focus starts. Pure, so every §6.3 state is a test
 * (`tests/popoverModel.test.ts`, one fixture per state in `tests/fixtures/popover/`).
 *
 * The precedence is Rust's (`popover_phase`, `surfaces/phase.rs`): this file reads `phase`, and under a Finder phase
 * `sync_phase` (task 2179), and never ranks the raw fields itself. On top of that it adds what the phase does not
 * carry: the 1887 send phase (shown over every state but "no session"), and the update and conflict rows, which sit
 * over a list state and never over a message (§6.3 "over any non-message state").
 *
 * Not built here: spec C's account states (checking, no plan yet, update required) and the notice line. Their data
 * is task 1747's Rust account view; `ACCOUNT_STATE_COPY` pins their words so that change only adds the data.
 */
import { FINDER_ACTION_LABEL, FINDER_ADDING_LINE, FINDER_REASON_COPY, type FinderSetupAction } from './finderSetupCopy'
import { accountInitial, monoReason, storageLine } from './macSettingsModel'
import type { ActivityRow, PopoverPhase, PopoverSnapshot } from './popoverContract'
import { P0_TITLE, P2_HINT, progressLine } from './signOutCopy'
import { formatStorageSize, systemLocale } from './storageFormat'

export { MACOS_DOWNLOAD_URL, UPDATE_DOWNLOAD_HINT, UPDATE_DOWNLOAD_LABEL } from './macUpdateCopy'
import { updateAvailableLine } from './macUpdateCopy'
import { updateOffer } from './macUpdateOffer'

/** The snapshot plus the two parts the popover reads beyond the reducer's phase (task 1683 slice 3, lead-approved). */
export type PopoverInput = PopoverSnapshot & {
  /** An update the updater offered, kept by Rust so a popover built later still sees it. */
  update?: { version: string; channel?: string | null } | null
  /** Task 1887's send phase, while it runs: the changes still waiting. */
  sign_out_sending?: { changes_left: number } | null
}

export type PopoverCommand =
  | 'open_review'
  | 'download_update'
  | 'resume'
  | 'retry_sync'
  | 'unlock'
  | 'add_storage'
  | 'open_beebeeb'
  | 'finder_try_again'
  | 'finder_open_system_settings'
  | 'finder_show_in_finder'
  | 'finder_copy_details'
  | 'open_location'
  | 'view_online'

export type PopoverIconName = 'pause' | 'play' | 'globe' | 'alert' | 'lock' | 'unlock' | 'db' | 'refresh' | 'sync'

export interface PopoverAction {
  label: string
  command: PopoverCommand
  /** The amber button: the one primary action of the view. */
  primary: boolean
}

export interface RowProgress {
  /** Null = unknown: no percentage, never a guess. */
  percent: number | null
  /** An upload is encryption happening, so its bar is amber; a download's is neutral. */
  encrypting: boolean
  /** The progressbar's value text (§11). */
  valueText: string
}

export interface RowView {
  id: string
  name: string
  meta: string
  icon: 'file' | 'image'
  direction: 'up' | 'down' | null
  progress: RowProgress | null
  failed: boolean
  /** The row as one sentence for VoiceOver (§11). */
  label: string
}

export type PopoverBody =
  | { kind: 'activity'; label: string; rows: RowView[] }
  | { kind: 'syncing'; left: string; total: string | null; percent: number | null; active: RowView[]; doneLabel: string | null; done: RowView[] }
  | {
      kind: 'message'
      tone: 'neutral' | 'error' | 'encryption' | 'busy'
      /** Null for the busy tone, which shows a spinner instead of a disc. */
      icon: PopoverIconName | null
      title: string
      lines: string[]
      /** The mono reason, at most 30 characters. */
      reason: string | null
      action: PopoverAction | null
    }

export interface PopoverView {
  kind: 'state'
  header: { email: string; initial: string; storage: { label: string; percent: number; red: boolean } | null }
  update: { line: string; hint: string; action: PopoverAction } | null
  attention: { title: string; text: string; action: PopoverAction } | null
  body: PopoverBody
  status: { kind: 'ok' | 'busy'; label: string; encrypted: boolean } | null
  /** Which footer actions can work right now (§6.4). */
  footer: { open_location: boolean; view_online: boolean; add_storage: boolean }
  /** Where focus starts (§11): the first primary action shown, else the gear. */
  focus: PopoverCommand | 'gear'
}

/** No session: there is no popover; a click opens the Beebeeb window on sign-in (§6.2). */
export type PopoverScreen = { kind: 'no_session' } | PopoverView

export interface PopoverViewOptions {
  locale?: string
  /** Minutes east of UTC, for "Yesterday". Defaults to this Mac's. */
  tzOffsetMinutes?: number
}

/** Spec C (C-R11, C-R5, C-R3), not built until task 1747: the words, pinned. */
export const ACCOUNT_STATE_COPY = {
  checking: { title: null, lines: [] as string[] },
  no_plan: {
    title: 'Choose a plan',
    lines: ['Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.'],
    waiting: 'Waiting for your plan.',
    action: 'Choose a plan on beebeeb.io',
  },
  update_required: {
    title: 'Update Beebeeb',
    lines: ['This version of Beebeeb is too old to connect. Update to continue.'],
    action: 'Update now',
  },
} as const

const ROWS = 5
const SYNCING_ROWS = 4
/** Each row on top (the update row, the attention row) takes the room of two activity rows. */
const ROWS_PER_TOP_ROW = 2

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)
const action = (label: string, command: PopoverCommand, primary = true): PopoverAction => ({ label, command, primary })

const FINDER_COMMAND: Readonly<Record<FinderSetupAction, PopoverCommand>> = {
  try_again: 'finder_try_again',
  open_system_settings: 'finder_open_system_settings',
  show_in_finder: 'finder_show_in_finder',
  copy_details: 'finder_copy_details',
}

const MONTHS_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const MONTHS_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']

/** "2 min ago" for the row, "2 minutes ago" for VoiceOver. Days are calendar days at `tzOffsetMinutes`. */
export function activityTime(at: number, nowSeconds: number, tzOffsetMinutes: number): { short: string; long: string } {
  const ago = Math.max(0, nowSeconds - at)
  if (ago < 60) return { short: 'Just now', long: 'just now' }
  if (ago < 3600) {
    const m = Math.floor(ago / 60)
    return { short: `${m} min ago`, long: `${m} ${plural(m, 'minute', 'minutes')} ago` }
  }
  const shift = tzOffsetMinutes * 60
  const day = (seconds: number) => Math.floor((seconds + shift) / 86400)
  const days = day(nowSeconds) - day(at)
  if (days === 0) {
    const h = Math.floor(ago / 3600)
    const words = `${h} ${plural(h, 'hour', 'hours')} ago`
    return { short: words, long: words }
  }
  if (days === 1) return { short: 'Yesterday', long: 'yesterday' }
  const date = new Date((at + shift) * 1000)
  return { short: `${date.getUTCDate()} ${MONTHS_SHORT[date.getUTCMonth()]}`, long: `${date.getUTCDate()} ${MONTHS_LONG[date.getUTCMonth()]}` }
}

const IMAGE = /\.(heic|heif|jpe?g|png|gif|webp|tiff?|bmp|raw|dng|cr2|nef|arw)$/i

function progressOf(row: ActivityRow): RowProgress {
  const encrypting = row.direction === 'up'
  const doing = encrypting ? 'encrypting and uploading' : 'downloading'
  const known = row.done_bytes !== null && row.total_bytes !== null && row.total_bytes > 0
  const percent = known ? Math.min(100, Math.round((row.done_bytes! / row.total_bytes!) * 100)) : null
  return { percent, encrypting, valueText: percent === null ? doing : `${percent} percent, ${doing}` }
}

function rowView(row: ActivityRow, nowSeconds: number, tz: number): RowView {
  const parts = row.folder ? [row.folder] : []
  const sentence = [row.name]
  let progress: RowProgress | null = null
  if (row.state === 'active') {
    progress = progressOf(row)
    const doing = progress.encrypting ? 'Encrypting and uploading' : 'Downloading'
    parts.push(doing)
    if (progress.percent !== null) parts.push(`${progress.percent} %`)
    sentence.push(doing.toLowerCase(), ...(row.folder ? [row.folder] : []), ...(progress.percent !== null ? [`${progress.percent} percent`] : []))
  } else if (row.state === 'failed') {
    parts.push('Couldn’t sync')
    sentence.push('couldn’t sync', ...(row.folder ? [row.folder] : []))
  } else {
    const when = activityTime(row.at, nowSeconds, tz)
    parts.push(when.short)
    sentence.push(row.direction === 'down' ? 'downloaded' : 'uploaded', ...(row.folder ? [row.folder] : []), when.long)
  }
  return {
    id: row.id,
    name: row.name,
    meta: parts.join(' · '),
    icon: IMAGE.test(row.name) ? 'image' : 'file',
    direction: row.state === 'failed' ? null : row.direction,
    progress,
    failed: row.state === 'failed',
    label: sentence.join(', '),
  }
}

function conflictRows(input: PopoverInput): RowView[] {
  return input.conflicts.files.map((file) => {
    const folder = input.activity.find((row) => row.id === file.file_id)?.folder ?? ''
    const meta = folder ? `${folder} · Changed on two devices` : 'Changed on two devices'
    return {
      id: `conflict-${file.file_id}`,
      name: file.file_name,
      meta,
      icon: IMAGE.test(file.file_name) ? 'image' : 'file',
      direction: null,
      progress: null,
      failed: false,
      label: [file.file_name, 'changed on two devices', ...(folder ? [folder] : [])].join(', '),
    }
  })
}

/** The sync part of the state: under a Finder phase it is `sync_phase` (Rust's phase with Finder taken as ready). */
function syncPart(input: PopoverInput): PopoverPhase {
  return input.phase.startsWith('finder_') ? input.sync_phase : input.phase
}

function messageBody(input: PopoverInput, phase: PopoverPhase): PopoverBody | null {
  const n = input.pending_changes
  switch (phase) {
    case 'paused':
      return {
        kind: 'message', tone: 'neutral', icon: 'pause', title: 'Sync is paused',
        lines: [n > 0
          ? `${n} ${plural(n, 'change on this Mac is', 'changes on this Mac are')} waiting. Nothing syncs until you resume.`
          : 'Nothing syncs until you resume.'],
        reason: null, action: action('Resume sync', 'resume'),
      }
    case 'offline':
      return {
        kind: 'message', tone: 'neutral', icon: 'globe', title: 'Can’t reach Beebeeb',
        lines: ['Check your connection. Files on this Mac are unchanged.'], reason: null, action: action('Try again', 'retry_sync'),
      }
    case 'error':
      return {
        kind: 'message', tone: 'error', icon: 'alert', title: 'Sync stopped',
        lines: ['Beebeeb couldn’t finish syncing. Files on this Mac are unchanged.'],
        reason: monoReason(input.reason?.code), action: action('Try again', 'retry_sync'),
      }
    case 'storage_full':
      return {
        kind: 'message', tone: 'error', icon: 'db', title: 'Your storage is full',
        lines: [`${n} ${plural(n, 'change', 'changes')} can’t upload until you free up space or add storage. They stay on this Mac.`],
        reason: null, action: action('Add storage', 'add_storage'),
      }
    case 'locked':
      return {
        kind: 'message', tone: 'encryption', icon: 'lock', title: 'Your vault is locked',
        lines: ['Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.'],
        reason: null, action: action('Unlock vault', 'unlock'),
      }
    case 'finder_failed':
    case 'finder_user_disabled': {
      const reason = input.finder.reason ?? (phase === 'finder_user_disabled' ? 'user_disabled' : 'unknown')
      const copy = FINDER_REASON_COPY[reason as keyof typeof FINDER_REASON_COPY] ?? FINDER_REASON_COPY.unknown
      return {
        kind: 'message', tone: 'error', icon: 'alert', title: 'Couldn’t add Beebeeb to Finder',
        lines: [copy.sentence], reason: input.finder.reason_line,
        action: action(FINDER_ACTION_LABEL[copy.action], FINDER_COMMAND[copy.action]),
      }
    }
    default:
      return null
  }
}

function listBody(input: PopoverInput, phase: PopoverPhase, topRows: number, nowSeconds: number, tz: number, locale: string): PopoverBody {
  const conflicts = conflictRows(input)
  const conflicted = new Set(input.conflicts.files.map((f) => f.file_id))
  const rows = input.activity.map((row) => rowView(row, nowSeconds, tz))
  if (phase === 'syncing') {
    const budget = Math.max(1, SYNCING_ROWS - ROWS_PER_TOP_ROW * topRows)
    const active = rows.filter((r) => r.progress !== null).slice(0, budget)
    const done = [...conflicts, ...rows.filter((r) => r.progress === null && !conflicted.has(r.id))].slice(0, budget - active.length)
    const left = input.engine.files_remaining ?? active.length
    const { bytes_total: total, bytes_done: doneBytes } = input.engine
    const percent = total !== null && doneBytes !== null && total > 0 ? Math.min(100, Math.round((doneBytes / total) * 100)) : null
    return {
      kind: 'syncing',
      left: `${left} ${plural(left, 'file', 'files')} left`,
      total: percent === null ? null : `${formatStorageSize(total, locale)} · ${percent} %`,
      percent,
      active,
      doneLabel: done.length > 0 ? 'Done' : null,
      done,
    }
  }
  const budget = Math.max(1, ROWS - ROWS_PER_TOP_ROW * topRows)
  return { kind: 'activity', label: 'Recent activity', rows: [...conflicts, ...rows.filter((r) => !conflicted.has(r.id))].slice(0, budget) }
}

export function popoverView(input: PopoverInput, nowSeconds: number, options: PopoverViewOptions = {}): PopoverScreen {
  if (input.phase === 'signed_out' || input.phase === 'session_ended') return { kind: 'no_session' }
  const locale = options.locale ?? systemLocale()
  const tz = options.tzOffsetMinutes ?? -new Date(nowSeconds * 1000).getTimezoneOffset()

  const storage = storageLine(input.storage, locale)
  const header = {
    email: input.account.email ?? '',
    initial: accountInitial(input.account.email),
    storage: input.phase === 'locked' || !storage ? null : { label: `Using ${storage.label}`, percent: storage.percent, red: storage.red },
  }
  const sync = syncPart(input)
  const offline = input.phase === 'offline' || sync === 'offline'
  const footer = { open_location: input.finder.setup === 'ready', view_online: !offline, add_storage: !offline }

  let body: PopoverBody | null = null
  if (input.sign_out_sending) {
    const counted = progressLine(input.sign_out_sending.changes_left)
    body = {
      kind: 'message', tone: 'busy', icon: null, title: P0_TITLE,
      lines: counted ? [counted, P2_HINT] : [P2_HINT], reason: null, action: action('Open Beebeeb', 'open_beebeeb', false),
    }
  } else if (input.phase === 'finder_adding' || input.phase === 'finder_missing') {
    body = messageBody(input, sync)
  } else {
    body = messageBody(input, input.phase)
  }

  let update: PopoverView['update'] = null
  let attention: PopoverView['attention'] = null
  let status: PopoverView['status'] = null
  if (body === null) {
    if (input.update) {
      const offer = updateOffer(input.update.channel, input.update.version)
      update = { line: updateAvailableLine(input.update.version), hint: offer.hint, action: action(offer.label, 'download_update') }
    }
    const count = input.conflicts.count
    if (count > 0) {
      attention = {
        title: `${count} ${plural(count, 'file needs', 'files need')} a decision`,
        text: 'Both changed on two devices.',
        action: action('Review…', 'open_review', false),
      }
    }
    const listPhase = sync === 'syncing' ? 'syncing' : 'synced'
    body = listBody(input, listPhase, (update ? 1 : 0) + (attention ? 1 : 0), nowSeconds, tz, locale)
    status = input.phase === 'finder_adding'
      ? { kind: 'busy', label: FINDER_ADDING_LINE, encrypted: false }
      : listPhase === 'syncing'
        ? { kind: 'busy', label: 'Syncing', encrypted: true }
        : { kind: 'ok', label: 'Up to date', encrypted: true }
  }

  const first = update?.action ?? attention?.action ?? (body.kind === 'message' ? body.action : null)
  return { kind: 'state', header, update, attention, body, status, footer, focus: first ? first.command : 'gear' }
}
