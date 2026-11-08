/**
 * Task 2409: the changes "Not sent" counts, one row each, for the Activity page's "Not sent" section, and their words.
 *
 * On Guus's PC Home said "Not sent 2" and the tray "2 changes waiting to sync", and no screen named the two changes.
 * `unsent_changes` (`src-tauri/src/unsent_changes_commands.rs`) lists exactly the rows the count counts, in the same
 * groups; this module narrows its answer and says, per row, what the change is, why it is not sent, and the one action
 * there is for it. Try again (Guus, 13:39) is the action of a change that failed: it stopped after repeated errors, or
 * it waits out a backoff after an error.
 *
 * Task 2414: the same rows serve "Not downloaded" (`not_downloaded`): a pin's downloads that are paused or out of
 * attempts, kind `download` (a file) or `folder` (a folder's pin), never with Try again.
 *
 * Copy rules: honest, never reassuring. A row names its file (this list is the place the names are; the polled
 * summary stays counts-only). Nothing here promises when a held change goes.
 */
import { BILLING_URL, command, type CommandResult } from '../desktopApi'

/** The kinds a row can be, as Rust names them (`state_db::unsent::change_kind`); `download` is a "Not downloaded" row. */
export type UnsentKind = 'new_file' | 'edit' | 'folder' | 'rename' | 'move' | 'delete' | 'restore' | 'download'

export interface UnsentChange {
  id: string
  file_id: string | null
  kind: UnsentKind
  /** Relative to the sync folder; for a backup, the backed-up file's own path on this PC. */
  path: string
  backup: boolean
  /** `waiting`, `waiting_for_folder`, `gave_up`, `not_marked_synced`, or a pause reason (`auth`, `quota`, …, `other`). */
  group: string
  /** Unix seconds it was queued (0 = not recorded). */
  queued_at: number
  /** Unix seconds a change that failed is tried again, while it waits out its backoff. */
  retry_at: number | null
  /** Given up because it parked (a stale base, a missing copy), not after repeated errors. */
  parked: boolean
  /** Try again applies. */
  retryable: boolean
  /**
   * Task 2414: a change that failed which Try again would not send (spec D8, the reconcile pass's checks):
   * `superseded` (a newer version of the file is already in Beebeeb) or `copy_gone` (the copy to send is gone).
   */
  blocked: string | null
  /** Being sent right now: bytes done of total. */
  sending: { done: number; total: number } | null
}

export interface UnsentChangesPage {
  total: number
  items: UnsentChange[]
  /** The engine's last state: `running` until its first sync check finished. */
  engine_state: string
  paused: boolean
}

/** How many rows the section shows at first, and adds per "Show more". */
export const UNSENT_PAGE_SIZE = 100

/** The most rows one `unsent_changes` call serves (`MAX_PAGE` in `src-tauri/src/unsent_changes_commands.rs`). */
export const UNSENT_MAX_PER_CALL = 500

/** The vault is locked: `unsent_changes`'s own gate, or (Windows) the session gate before it ("… or stopping"). */
export function isLockedRefusal(reason: string): boolean {
  return reason.startsWith('vault is locked')
}

const KINDS: ReadonlySet<string> = new Set(['new_file', 'edit', 'folder', 'rename', 'move', 'delete', 'restore', 'download'])

type Obj = Record<string, unknown>
const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)
const isStrOrNull = (v: unknown): v is string | null => v === null || typeof v === 'string'

function isChange(v: unknown): v is UnsentChange {
  if (!isObj(v)) return false
  const sending = v.sending
  return (
    typeof v.id === 'string' &&
    isStrOrNull(v.file_id) &&
    typeof v.kind === 'string' &&
    KINDS.has(v.kind) &&
    typeof v.path === 'string' &&
    typeof v.backup === 'boolean' &&
    typeof v.group === 'string' &&
    isNum(v.queued_at) &&
    (v.retry_at === null || isNum(v.retry_at)) &&
    typeof v.parked === 'boolean' &&
    typeof v.retryable === 'boolean' &&
    isStrOrNull(v.blocked) &&
    (sending === null || (isObj(sending) && isCount(sending.done) && isCount(sending.total)))
  )
}

/** Narrow an `unsent_changes` answer. Null when it is not the contract's shape: the section then says it could not read. */
export function parseUnsentChanges(raw: unknown): UnsentChangesPage | null {
  if (!isObj(raw) || !isCount(raw.total) || !Array.isArray(raw.items) || !raw.items.every(isChange)) return null
  if (typeof raw.engine_state !== 'string' || typeof raw.paused !== 'boolean') return null
  return raw as unknown as UnsentChangesPage
}

/** The two lists these rows serve: "Not sent" and (task 2414) "Not downloaded". */
export type UnsentListCommand = 'unsent_changes' | 'not_downloaded'

/**
 * The first `count` rows (the rows shown) of `name`'s list. Up to `UNSENT_MAX_PER_CALL` that is one call, so one read transaction: the
 * count and the rows agree, and no row repeats or goes missing. Past it, further calls; a change that moved between two
 * calls can shift the order, so a row already read is not added twice (the next poll reads it right). One answer with
 * the total and engine state of the first call; a call that fails fails the read.
 */
export async function loadUnsentChanges(
  count: number,
  call: typeof command = command,
  name: UnsentListCommand = 'unsent_changes',
): Promise<CommandResult<UnsentChangesPage>> {
  let first: UnsentChangesPage | null = null
  const items: UnsentChange[] = []
  const seen = new Set<string>()
  const want = Math.max(count, 1)
  for (let offset = 0; offset < want; offset += UNSENT_MAX_PER_CALL) {
    const limit = Math.min(want - offset, UNSENT_MAX_PER_CALL)
    const result = await call<unknown>(name, { offset, limit })
    if (!result.ok) return result
    const next = parseUnsentChanges(result.value)
    if (!next) {
      const what = name === 'not_downloaded' ? 'files not downloaded' : 'changes not sent'
      return { ok: false, reason: `The list of ${what} could not be read.`, unsupported: false }
    }
    first ??= next
    for (const item of next.items) {
      if (seen.has(item.id)) continue
      seen.add(item.id)
      items.push(item)
    }
    if (offset + limit >= next.total) break
  }
  return { ok: true, value: { ...(first as UnsentChangesPage), items } }
}

/**
 * Try again on one change. `queued` on success; a refusal's code (`unknown`, `held`, `parked`, `superseded`,
 * `copy_gone`) as the reason.
 */
export function retryUnsentChange(id: string, call: typeof command = command): Promise<CommandResult<string>> {
  return call<string>('retry_unsent_change', { id })
}

/** The words for a refused Try again. */
export function retryRefusal(reason: string): string {
  switch (reason) {
    case 'unknown':
      return 'It isn’t waiting any more: it was sent or removed in the meantime.'
    case 'held':
      return 'It’s on hold, so it can’t be tried again. Its own action is shown.'
    case 'parked':
      return 'Beebeeb can’t send this one as it is, so trying again won’t help.'
    case 'superseded':
      return 'A newer version of this file is already in Beebeeb, so this change isn’t sent over it. Nothing was overwritten.'
    case 'copy_gone':
      return 'The copy to send is gone, so trying again can’t send it.'
    default:
      return reason
  }
}

// ── Words ───────────────────────────────────────────────────────────────────

const KIND_WORD: Record<UnsentKind, string> = {
  new_file: 'New file',
  edit: 'Edit',
  folder: 'Folder',
  rename: 'Rename',
  move: 'Move',
  delete: 'Delete',
  restore: 'Restore',
  download: 'Download',
}

export function kindWord(kind: UnsentKind): string {
  return KIND_WORD[kind]
}

/** The last part of a path: the one shared helper (`src/pathParts.ts`). */
export { baseName } from '../pathParts'

/** Everything before the last part, as written; "" at the top of the sync folder. */
export function folderOf(path: string): string {
  const at = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return at > 0 ? path.slice(0, at) : ''
}

/** "14:05", or "9 Nov 14:05" on another day. Local time. */
export function retryTime(unixSeconds: number, now: Date = new Date()): string {
  const d = new Date(unixSeconds * 1000)
  const hhmm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  const sameDay = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate()
  if (sameDay) return hhmm
  const month = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.getMonth()]
  return `${d.getDate()} ${month} ${hhmm}`
}

export interface UnsentContext {
  engineState: string
  paused: boolean
  now?: Date
}

/** Why one change is not sent, and whether the line needs the person (`attention`: the warn colour). */
export function unsentReason(item: UnsentChange, ctx: UnsentContext): { text: string; attention: boolean } {
  const now = ctx.now ?? new Date()
  const calm = (text: string) => ({ text, attention: false })
  const needs = (text: string) => ({ text, attention: true })
  if (item.sending) {
    const { done, total } = item.sending
    return calm(total > 0 ? `Sending · ${Math.min(100, Math.floor((done / total) * 100))}%` : 'Sending')
  }
  // Task 2414: a change that failed and that Try again would not send says why (it has no Try again). One still waiting
  // out a backoff keeps its retry line: the engine tries it again at that time all the same (self-review).
  if (item.blocked !== null && item.retry_at === null) {
    if (item.blocked === 'copy_gone') return needs('Can’t send: the copy to send is gone.')
    if (item.blocked === 'superseded') {
      return needs(
        item.kind === 'new_file'
          ? 'Can’t send: this file is already in Beebeeb. Nothing was overwritten.'
          : 'Can’t send: a newer version of this file is already in Beebeeb. Nothing was overwritten.',
      )
    }
    return needs('Can’t send it as it is.')
  }
  switch (item.group) {
    case 'waiting':
      if (item.retry_at !== null) return needs(`Couldn’t send; trying again at ${retryTime(item.retry_at, now)}`)
      if (ctx.paused) return calm('Waits until you resume sync')
      if (ctx.engineState === 'running') return calm('Waiting for the first sync check')
      return calm('Queued')
    case 'waiting_for_folder':
      return calm('Waiting for its folder to reach Beebeeb')
    case 'gave_up':
      return item.parked
        ? needs('Stopped: Beebeeb can’t send it as it is. Your copy stays on this PC.')
        : needs('Stopped after repeated errors')
    case 'auth':
      return needs('On hold until you sign in again')
    case 'quota':
      return needs('On hold: your storage is full')
    case 'permission':
      return needs('On hold: you don’t have permission to make this change')
    case 'locked':
      return calm('On hold while Beebeeb is locked')
    case 'key_replaced':
      return needs('On hold: made with a key this account no longer uses')
    case 'found_while_paused':
      return needs('Found while sync was paused; not uploaded until you decide')
    case 'needs_review':
      return needs(
        'On hold: an item with the same name is already in Beebeeb. Nothing was overwritten or deleted. Contact support@beebeeb.io and we’ll sort it out with you.',
      )
    case 'not_marked_synced':
      return needs('Uploaded, but this PC couldn’t mark it as synced. Contact support@beebeeb.io and we’ll sort it out with you.')
    default:
      return needs('On hold')
  }
}

/**
 * Task 2414: why a "Not downloaded" row did not download. A pin's downloads are paused only by what pauses the whole
 * queue (signed out, locked); the upload reasons ("storage full", "found while paused", …) are never theirs, so any
 * other pause is "On hold", never an upload's sentence (self-review).
 */
export function notDownloadedReason(item: UnsentChange): { text: string; attention: boolean } {
  switch (item.group) {
    case 'gave_up':
      return { text: 'Stopped after repeated errors', attention: true }
    case 'auth':
      return { text: 'On hold until you sign in again', attention: true }
    case 'locked':
      return { text: 'On hold while Beebeeb is locked', attention: false }
    default:
      return { text: 'On hold', attention: true }
  }
}

/** Task 2414: a "Not downloaded" row's one action: Sign in again, for one held until then; nothing else applies. */
export function notDownloadedAction(item: UnsentChange): UnsentAction | null {
  return item.group === 'auth' ? { kind: 'sign_in_again', label: 'Sign in again' } : null
}

export type UnsentAction =
  | { kind: 'retry'; label: 'Try again' }
  | { kind: 'sign_in_again'; label: 'Sign in again' }
  | { kind: 'add_storage'; label: 'Add storage'; href: string }
  | { kind: 'open_sync_settings'; label: 'Decide in Settings › Sync' }

/** The one action a row has, if any. A held row keeps its own and never gets Try again. */
export function unsentAction(item: UnsentChange): UnsentAction | null {
  if (item.retryable && !item.sending) return { kind: 'retry', label: 'Try again' }
  switch (item.group) {
    case 'auth':
      return { kind: 'sign_in_again', label: 'Sign in again' }
    case 'quota':
      return { kind: 'add_storage', label: 'Add storage', href: BILLING_URL }
    case 'found_while_paused':
      return { kind: 'open_sync_settings', label: 'Decide in Settings › Sync' }
    default:
      return null
  }
}

/** A row's action in flight, or a Try again that worked: the row reads "Queued" until a read shows it moving. */
export type RowState = 'busy' | 'queued'

/**
 * The row states that still hold after a read (2409 review P3: the read right after Try again cleared "Queued" at once,
 * and the row then said "Waiting for the first sync check"). An action in flight stays. "Queued" stays while the read
 * shows the change still waiting its turn: listed, waiting, not failed again, not being sent. Once it moves (sent,
 * sending, failed again, held, or off the page), the read words it.
 */
export function keepRowStates(states: Record<string, RowState>, items: UnsentChange[]): Record<string, RowState> {
  const byId = new Map(items.map((item) => [item.id, item]))
  const waitingItsTurn = (id: string) => {
    const item = byId.get(id)
    return !!item && item.group === 'waiting' && item.retry_at === null && !item.sending && !item.retryable && item.blocked === null
  }
  return Object.fromEntries(Object.entries(states).filter(([id, state]) => state === 'busy' || waitingItsTurn(id)))
}

/** "just now", "12 min ago", "3 hr ago", "4 days ago"; "" when not recorded. */
export function queuedAgo(unixSeconds: number, now: Date = new Date()): string {
  if (unixSeconds <= 0) return ''
  const sec = Math.max(0, Math.round(now.getTime() / 1000 - unixSeconds))
  if (sec < 45) return 'just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr} hr ago`
  const day = Math.round(hr / 24)
  return `${day} day${day === 1 ? '' : 's'} ago`
}
