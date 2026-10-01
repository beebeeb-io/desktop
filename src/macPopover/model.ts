/**
 * The menu-bar popover's view model (task 1683 slice 3; spec
 * `docs/specs/2026-09-30-macos-menubar-popover.md` sections 3, 4 and 9).
 *
 * ONE pure function, `viewForSnapshot`, turns what Rust says (`PopoverSnapshot`, slice 2) plus
 * the little the window itself knows (`ViewContext`) into what is drawn: which of the spec's
 * states this is, the header, the body, the status row, the footer and the exact copy.
 * `MacPopover.tsx` only paints this. Nothing here reads the network, a clock or the DOM, so
 * the whole state-to-view mapping is a bun test (`tests/macPopoverModel.test.ts`).
 *
 * Who decides the state: Rust. `snapshot.phase` is the spec's precedence reducer
 * (`surfaces/phase.rs`); this file never re-derives it from raw fields. It only adds what the
 * reducer cannot know: a list state that has nothing to list (a2), a list state with a
 * conflict row (a1), per-file progress being known or not (b / b'), and the three things the
 * window itself is doing (the unlock form is open, a Finder install is in flight, the last
 * action failed).
 *
 * The one rule that prevents "said twice" (spec section 3): a state that needs an explanation
 * or an action is a MESSAGE state, the body carries it and the status row is dropped. The
 * status row exists only in the list states (a, a1, a2, b, b').
 */
import { formatStorageSize } from '../storageFormat'
import type { ActivityRow, PopoverPhase, PopoverSnapshot } from '../popoverContract'

/**
 * The state ids are the design file's (`design/hifi/macos-menubar-popover.jsx` `mpSpec`), so a
 * render here and the design's render can be put side by side by name.
 * Spec letters: synced a, conflict a1, empty a2, syncing b, syncing0 b', locked c1, unlock c1b,
 * paused c2, error d, offline d2, storage s, signedout e, ended e2, finder f, finderadding f1,
 * finderfail f2. Two ids are NOT in the approved design and are flagged as such: `finderoff`
 * (the phase slice 2 added by lead ruling, never drawn) and `loadfail` (the snapshot command
 * itself failed). `loading` is the first frame, before any snapshot has arrived.
 */
export type PopoverStateId =
  | 'loading'
  | 'loadfail'
  | 'synced'
  | 'conflict'
  | 'empty'
  | 'syncing'
  | 'syncing0'
  | 'locked'
  | 'unlock'
  | 'paused'
  | 'error'
  | 'offline'
  | 'storage'
  | 'signedout'
  | 'ended'
  | 'finder'
  | 'finderadding'
  | 'finderfail'
  | 'finderoff'

/** The ids that carry a message body (no status row): everything except the list states. */
export const LIST_STATES: readonly PopoverStateId[] = ['synced', 'conflict', 'syncing', 'syncing0']

export type IconName =
  | 'alert' | 'pause' | 'play' | 'globe' | 'drive' | 'unlock' | 'external' | 'refresh' | 'wifiOff'
  | 'arrowUp' | 'arrowDown' | 'versions' | 'fingerprint' | 'lock' | 'folder' | 'file' | 'image'
  | 'settings' | 'check' | 'shield' | 'users' | 'user'

/** Fixed 64 pt art slot content, one style (spec section 3 "Design decisions"). */
export type ArtId = 'folderEnc' | 'lock' | 'pause' | 'alert' | 'wifiOff' | 'drive' | 'shield' | 'users' | 'folderDashed' | 'spinnerDashed'

/** What a button does. The controller maps each to a command (`macPopover/controller.ts`). */
export type ActionId =
  | 'unlock_open' | 'unlock_submit' | 'resume' | 'retry' | 'add_storage' | 'setup' | 'sign_in'
  | 'finder_add' | 'finder_retry' | 'open_system_settings' | 'reload'

export interface ActionView {
  id: ActionId
  label: string
  icon: IconName | null
  /** Drawn but not pressable (f1's `Adding…`). */
  disabled: boolean
}

export type HeaderView =
  | { mode: 'brand' }
  | {
      mode: 'account'
      email: string | null
      /** `84,3 GB of 200 GB`; the UI prefixes `Using `. Null: no figure to show (locked, not loaded). */
      usage: string | null
      /** 0-100 and rounded, null when there is no storage figure. */
      pct: number | null
      /** The bar turns red from 90 %. */
      full: boolean
    }

export type StatusView = { kind: 'ok' | 'sync'; text: string; /** Tooltip: `Last checked 2 min ago`. */ title: string | null }

export interface ActivityItemView {
  kind: 'activity'
  id: string
  name: string
  tile: 'file' | 'image'
  /** `Investigations · 2 min ago`, `Photos · Downloading`, ... */
  subline: string
  direction: 'up' | 'down' | null
  failed: boolean
  /** 0-100 for a per-file bar, null for no bar (spec b': never a guessed percentage). */
  progress: number | null
  /** Amber only for bytes being ENCRYPTED (uploads); downloads are decrypted, neutral. */
  progressTone: 'encrypting' | 'neutral'
  /** The row's tooltip (the path inside the vault). */
  title: string
  /** What VoiceOver reads for the whole row. */
  label: string
}

export interface ConflictItemView {
  kind: 'conflict'
  title: string
  subline: string
  /** `Review` opens the first file. */
  fileId: string
  fileName: string
}

export type ListItem = { kind: 'label'; text: string } | ActivityItemView | ConflictItemView

export interface SummaryView {
  /** `14 files left`. */
  left: string
  /** `1,2 GB · 38 %`, or null when the byte totals are not known. */
  right: string | null
  /** 0-100, or null for an indeterminate bar (spec: never fake a percentage). */
  pct: number | null
}

export type BodyView =
  | { kind: 'list'; summary: SummaryView | null; items: ListItem[] }
  | {
      kind: 'message'
      art: ArtId
      artTone: 'neutral' | 'amber' | 'err'
      title: string
      copy: string | null
      /** The mono reason line: at most 30 characters, adds information, never restates the copy. */
      detail: string | null
      action: ActionView | null
      /** The password form (c1b). */
      form: { wrong: boolean } | null
    }
  /** The first frame, before any snapshot: the frame and nothing that claims a state. */
  | { kind: 'blank' }

export interface PopoverView {
  state: PopoverStateId
  header: HeaderView
  body: BodyView
  status: StatusView | null
  footer: { visible: boolean; finderDisabled: boolean }
}

export interface ViewContext {
  /** Unix seconds. The model never reads a clock. */
  now: number
  /** BCP 47 tag from the Mac (`navigator.languages[0]`). */
  locale: string
  /** c1 -> c1b: the user pressed `Unlock vault`. */
  unlockOpen: boolean
  /** c1b: the last password was refused. */
  unlockWrong: boolean
  /** A Finder install this window started is still running (f1 on screen at once). */
  finderPending: boolean
}

export const DEFAULT_CONTEXT: Omit<ViewContext, 'now' | 'locale'> = {
  unlockOpen: false,
  unlockWrong: false,
  finderPending: false,
}

// ── Copy (spec section 4, exact; ’ stands for the typographic apostrophe) ───────────────────

export const COPY = {
  recentLabel: 'Recent activity',
  decisionLabel: 'Needs your decision',
  doneLabel: 'Done',
  upToDate: 'Up to date',
  syncing: 'Syncing',
  encrypted: 'Encrypted',
  conflictOne: 'Two versions. Choose which to keep.',
  conflictMany: 'Choose which to keep.',
  review: 'Review',
  uploading: 'Encrypting and uploading',
  downloading: 'Downloading',
  failedRow: 'Couldn’t sync',
  wrongPassword: 'That password didn’t work. Try again.',
} as const

const MONO_MAX = 30

/** The mono reason line: single line, at most 30 characters, truncated with an ellipsis. */
export function monoReason(line: string | null | undefined): string | null {
  const text = line?.replace(/\s+/g, ' ').trim()
  if (!text) return null
  return text.length <= MONO_MAX ? text : `${text.slice(0, MONO_MAX - 1)}…`
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

// ── Dates and sizes ────────────────────────────────────────────────────────────────────────

function startOfLocalDay(ms: number): number {
  const d = new Date(ms)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
}

/**
 * `just now`, `2 min ago`, `1 hour ago`, `Yesterday`, `3 days ago`, then a date in the Mac's
 * locale. `spoken` writes the units out for VoiceOver (`2 minutes ago`).
 */
export function formatRecency(now: number, at: number, locale: string, spoken = false): string {
  if (!Number.isFinite(at) || at <= 0) return ''
  const seconds = Math.max(0, now - at)
  if (seconds < 45) return 'just now'
  const minutes = Math.max(1, Math.round(seconds / 60))
  if (minutes < 60) return `${minutes} ${spoken ? plural(minutes, 'minute', 'minutes') : 'min'} ago`
  const dayDiff = Math.round((startOfLocalDay(now * 1000) - startOfLocalDay(at * 1000)) / 86_400_000)
  if (dayDiff <= 0) {
    const hours = Math.max(1, Math.floor(seconds / 3600))
    return `${hours} ${plural(hours, 'hour', 'hours')} ago`
  }
  if (dayDiff === 1) return 'Yesterday'
  if (dayDiff < 7) return `${dayDiff} days ago`
  const sameYear = new Date(now * 1000).getFullYear() === new Date(at * 1000).getFullYear()
  const options: Intl.DateTimeFormatOptions = sameYear ? { day: 'numeric', month: 'short' } : { day: 'numeric', month: 'short', year: 'numeric' }
  try {
    return new Intl.DateTimeFormat(locale, options).format(at * 1000)
  } catch {
    return new Intl.DateTimeFormat('en', options).format(at * 1000)
  }
}

const percent = (done: number, total: number): number => Math.min(100, Math.max(0, Math.round((done / total) * 100)))

// ── Pieces ─────────────────────────────────────────────────────────────────────────────────

function headerFor(snapshot: PopoverSnapshot, locale: string): HeaderView {
  if (snapshot.phase === 'signed_out' || snapshot.phase === 'session_ended') return { mode: 'brand' }
  const storage = snapshot.phase === 'locked' ? null : snapshot.storage
  const quota = storage?.quota_bytes ?? 0
  const known = storage !== null && quota > 0
  return {
    mode: 'account',
    email: snapshot.account.email,
    usage: known ? `${formatStorageSize(storage.used_bytes, locale)} of ${formatStorageSize(quota, locale)}` : null,
    pct: known ? percent(storage.used_bytes, quota) : null,
    full: known && percent(storage.used_bytes, quota) >= 90,
  }
}

const tileFor = (name: string): 'file' | 'image' => (/\.(jpe?g|png|gif|heic|heif|webp|svg|ico|bmp|tiff?|avif|dng|cr[23]|nef|arw|orf|rw2|raf)$/i.test(name) ? 'image' : 'file')

function activityItem(row: ActivityRow, ctx: ViewContext): ActivityItemView {
  const failed = row.state === 'failed'
  const active = row.state === 'active'
  const verb = row.direction === 'down' ? COPY.downloading : COPY.uploading
  const known = active && row.done_bytes !== null && row.total_bytes !== null && row.total_bytes > 0
  const when = formatRecency(ctx.now, row.at, ctx.locale)
  const what = failed ? COPY.failedRow : active ? verb : when
  const spokenWhat = failed ? COPY.failedRow : active ? verb : formatRecency(ctx.now, row.at, ctx.locale, true)
  const progress = known ? percent(row.done_bytes as number, row.total_bytes as number) : null
  const direction = row.direction === 'up' || row.direction === 'down' ? row.direction : null
  const directionWord = direction === 'up' ? 'uploaded' : direction === 'down' ? 'downloaded' : ''
  return {
    kind: 'activity',
    id: row.id,
    name: row.name,
    tile: tileFor(row.name),
    subline: [row.folder, what, progress !== null ? `${progress} %` : ''].filter(Boolean).join(' · '),
    direction,
    failed,
    progress,
    progressTone: row.direction === 'up' ? 'encrypting' : 'neutral',
    title: row.path,
    label: [row.name, row.folder, spokenWhat, progress !== null ? `${progress} percent` : '', active ? '' : directionWord].filter(Boolean).join(', '),
  }
}

function conflictItem(snapshot: PopoverSnapshot): ConflictItemView | null {
  const { count, files } = snapshot.conflicts
  if (count <= 0 || files.length === 0) return null
  const first = files[0]
  return count === 1
    ? { kind: 'conflict', title: first.file_name, subline: COPY.conflictOne, fileId: first.file_id, fileName: first.file_name }
    : { kind: 'conflict', title: `${count} files have two versions.`, subline: COPY.conflictMany, fileId: first.file_id, fileName: first.file_name }
}

function summaryFor(snapshot: PopoverSnapshot, locale: string): SummaryView | null {
  const left = snapshot.engine.files_remaining
  if (left === null || left <= 0) return null
  const { bytes_total: total, bytes_done: done } = snapshot.engine
  const known = total !== null && done !== null && total > 0
  const pct = known ? percent(done, total) : null
  return {
    left: `${left} ${plural(left, 'file', 'files')} left`,
    right: known ? `${formatStorageSize(total, locale)} · ${pct} %` : null,
    pct,
  }
}

function listState(snapshot: PopoverSnapshot, ctx: ViewContext): { state: PopoverStateId; body: BodyView } {
  const conflict = conflictItem(snapshot)
  const rows = snapshot.activity.map((row) => activityItem(row, ctx))
  const items: ListItem[] = []
  if (conflict) items.push({ kind: 'label', text: COPY.decisionLabel }, conflict)

  if (snapshot.phase === 'syncing') {
    const active = rows.filter((_, i) => snapshot.activity[i].state === 'active')
    const rest = rows.filter((_, i) => snapshot.activity[i].state !== 'active')
    items.push(...active)
    if (rest.length > 0) {
      const allDone = rest.every((r) => !r.failed)
      items.push({ kind: 'label', text: allDone ? COPY.doneLabel : COPY.recentLabel }, ...rest)
    }
    const perFile = snapshot.activity.some((r) => r.state === 'active' && r.done_bytes !== null && r.total_bytes !== null)
    return { state: perFile ? 'syncing' : 'syncing0', body: { kind: 'list', summary: summaryFor(snapshot, ctx.locale), items } }
  }

  if (rows.length === 0 && !conflict) {
    return { state: 'empty', body: message('folderEnc', 'neutral', 'Nothing new yet', 'Changes on this Mac and in your vault appear here.') }
  }
  if (rows.length > 0) items.push({ kind: 'label', text: COPY.recentLabel }, ...rows)
  return { state: conflict ? 'conflict' : 'synced', body: { kind: 'list', summary: null, items } }
}

const action = (id: ActionId, label: string, icon: IconName | null, disabled = false): ActionView => ({ id, label, icon, disabled })

function message(
  art: ArtId,
  artTone: 'neutral' | 'amber' | 'err',
  title: string,
  copy: string | null,
  extra: { detail?: string | null; action?: ActionView | null; form?: { wrong: boolean } | null } = {},
): BodyView {
  return { kind: 'message', art, artTone, title, copy, detail: extra.detail ?? null, action: extra.action ?? null, form: extra.form ?? null }
}

const changes = (n: number, one: string, many: string) => (n === 1 ? one : many)

function messageState(snapshot: PopoverSnapshot, ctx: ViewContext): { state: PopoverStateId; body: BodyView } {
  const phase: PopoverPhase = snapshot.phase
  const pending = snapshot.pending_changes
  // A Finder install this window started is running: show f1 now, not after the backend says so.
  const finderPhase = ctx.finderPending && (phase === 'finder_missing' || phase === 'finder_failed') ? 'finder_adding' : phase

  switch (finderPhase) {
    case 'signed_out':
      return {
        state: 'signedout',
        body: message('shield', 'neutral', 'Welcome to Beebeeb', 'Create an account or sign in to start syncing. Files are encrypted on this Mac first, then stored in the EU.', { action: action('setup', 'Set up Beebeeb', null) }),
      }
    case 'session_ended':
      return {
        state: 'ended',
        body: message('users', 'neutral', 'Your session ended', 'Sign in again to keep syncing. Files on this Mac are unchanged.', { action: action('sign_in', 'Sign in', null) }),
      }
    case 'locked':
      return ctx.unlockOpen
        ? { state: 'unlock', body: message('lock', 'amber', 'Enter your password', null, { form: { wrong: ctx.unlockWrong }, action: action('unlock_submit', 'Unlock', null) }) }
        : {
            state: 'locked',
            body: message('lock', 'amber', 'Your vault is locked', 'Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.', { action: action('unlock_open', 'Unlock vault', 'unlock') }),
          }
    case 'finder_failed': {
      // The approved sentence is about a timeout. Every other category gets a sentence that
      // claims nothing about time (slice 2 amendment: the mono line shows every category).
      const timeout = snapshot.finder.reason === 'timeout' || snapshot.finder.reason === null
      return {
        state: 'finderfail',
        body: message('alert', 'err', 'Couldn’t add Beebeeb to Finder', timeout ? 'macOS didn’t respond in time.' : 'macOS couldn’t finish adding it.', {
          detail: monoReason(snapshot.finder.reason_line),
          action: action('finder_retry', 'Try again', 'refresh'),
        }),
      }
    }
    case 'finder_user_disabled':
      return {
        state: 'finderoff',
        body: message('folderDashed', 'neutral', 'Beebeeb is turned off in Finder', 'You turned it off in System Settings. Turn it back on to see your files in Finder.', { action: action('open_system_settings', 'Open System Settings', 'external') }),
      }
    case 'finder_adding':
      return {
        state: 'finderadding',
        body: message('spinnerDashed', 'neutral', 'Adding Beebeeb to Finder', 'macOS is setting it up.', { action: action('finder_add', 'Adding…', null, true) }),
      }
    case 'finder_missing':
      return {
        state: 'finder',
        body: message('folderDashed', 'neutral', 'Beebeeb isn’t in Finder yet', 'Add it to see your files in Finder like any other folder. Files download when you open them, unless you keep them on this Mac.', { action: action('finder_add', 'Add to Finder', null) }),
      }
    case 'paused':
      return {
        state: 'paused',
        body: message(
          'pause',
          'neutral',
          'Sync is paused',
          pending > 0
            ? `${pending} ${changes(pending, 'change on this Mac is', 'changes on this Mac are')} waiting. Nothing syncs until you resume.`
            : 'Nothing syncs until you resume.',
          { action: action('resume', 'Resume sync', 'play') },
        ),
      }
    case 'storage_full':
      return {
        state: 'storage',
        body: message(
          'drive',
          'err',
          'Your storage is full',
          pending > 0
            ? `${pending} ${changes(pending, 'change can’t', 'changes can’t')} upload until you free up space or add storage. ${changes(pending, 'It stays', 'They stay')} on this Mac.`
            : 'New changes can’t upload until you free up space or add storage. They stay on this Mac.',
          { action: action('add_storage', 'Add storage', null) },
        ),
      }
    case 'offline':
      return {
        state: 'offline',
        body: message('wifiOff', 'neutral', 'You’re offline', 'Changes on this Mac will sync when you’re back online. Beebeeb keeps checking.'),
      }
    case 'error':
      return {
        state: 'error',
        body: message('alert', 'err', 'Can’t reach Beebeeb', 'Beebeeb didn’t answer. Files on this Mac are unchanged.', {
          detail: monoReason(snapshot.reason?.detail),
          action: action('retry', 'Try again', 'refresh'),
        }),
      }
    default:
      // Not a message phase (synced and syncing are list states, handled by the caller). A phase
      // added to the Rust enum lands here, which is why the test walks `POPOVER_PHASES`.
      return { state: 'loadfail', body: loadFailBody() }
  }
}

function loadFailBody(): BodyView {
  return message('alert', 'err', 'Can’t read Beebeeb’s status', 'Something went wrong inside the app. Try again.', { action: action('reload', 'Try again', 'refresh') })
}

const MESSAGE_PHASES: readonly PopoverPhase[] = [
  'signed_out', 'session_ended', 'locked', 'finder_failed', 'finder_user_disabled', 'finder_adding', 'finder_missing', 'paused', 'storage_full', 'offline', 'error',
]

function statusFor(snapshot: PopoverSnapshot, state: PopoverStateId, ctx: ViewContext): StatusView | null {
  if (state === 'syncing' || state === 'syncing0') return { kind: 'sync', text: COPY.syncing, title: null }
  if (state === 'synced' || state === 'conflict' || state === 'empty') {
    const at = snapshot.engine.last_tick_ok_at
    const when = at === null ? '' : formatRecency(ctx.now, at, ctx.locale)
    return { kind: 'ok', text: COPY.upToDate, title: when ? `Last checked ${when}` : null }
  }
  return null
}

/** The popover before any snapshot has arrived: the frame, nothing that claims a state. */
export function loadingView(): PopoverView {
  // The brand header: with no snapshot the account is unknown, so no name, no avatar, no figure.
  return { state: 'loading', header: { mode: 'brand' }, body: { kind: 'blank' }, status: null, footer: { visible: false, finderDisabled: true } }
}

/** The snapshot command failed and there is no earlier snapshot to keep showing. */
export function loadFailView(): PopoverView {
  return { state: 'loadfail', header: { mode: 'brand' }, body: loadFailBody(), status: null, footer: { visible: false, finderDisabled: true } }
}

export function viewForSnapshot(snapshot: PopoverSnapshot, ctx: ViewContext): PopoverView {
  // Only the two list phases draw a list. Anything else the reducer does not name (a phase added in
  // Rust before this file learns it) is the load-failed state, never a guess at "Up to date".
  const picked = MESSAGE_PHASES.includes(snapshot.phase)
    ? messageState(snapshot, ctx)
    : snapshot.phase === 'synced' || snapshot.phase === 'syncing'
      ? listState(snapshot, ctx)
      : { state: 'loadfail' as const, body: loadFailBody() }
  const signedOutSurface = snapshot.phase === 'signed_out' || snapshot.phase === 'session_ended'
  return {
    state: picked.state,
    header: headerFor(snapshot, ctx.locale),
    body: picked.body,
    status: statusFor(snapshot, picked.state, ctx),
    // `Open in Finder` is the one footer action that can be unavailable: not while Finder is not set up.
    footer: { visible: !signedOutSurface, finderDisabled: snapshot.finder.setup !== 'ready' || ctx.finderPending },
  }
}
