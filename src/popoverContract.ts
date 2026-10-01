/**
 * The contract between the Rust side and the menu-bar popover (task 1683 slice 2;
 * spec `docs/specs/2026-09-30-macos-menubar-popover.md` sections 6 (rule 9) and 9).
 *
 * Nothing here is rendered yet: slice 3 builds the popover on these types, slice 6
 * flips macOS to it. The old UI neither imports nor listens to any of it.
 *
 * Every shape below is the serialized form of a Rust type in
 * `src-tauri/src/popover_data.rs` or `engine_status.rs`. `tests/popoverContract.test.ts`
 * keeps the two honest: it checks the phase list against the Rust enum's source and
 * runs the validator over the JSON fixtures the Rust tests compare their output to.
 */

/** Event Rust emits when the popover is shown; the UI refreshes its snapshot then. */
export const POPOVER_SHOWN_EVENT = 'popover-shown'
/** Event that points the single `review` window at another file. */
export const REVIEW_OPEN_EVENT = 'review:open'
/** The engine's status stream (unchanged name). */
export const ENGINE_STATUS_EVENT = 'engine-status'

/** Precedence order, highest first (spec section 3). Mirrors `PopoverPhase` in `surfaces/phase.rs`. */
export const POPOVER_PHASES = [
  'signed_out',
  'session_ended',
  'locked',
  'finder_failed',
  'finder_user_disabled',
  'finder_adding',
  'finder_missing',
  'paused',
  'storage_full',
  'offline',
  'error',
  'syncing',
  'synced',
] as const
export type PopoverPhase = (typeof POPOVER_PHASES)[number]

export const FINDER_SETUPS = ['ready', 'missing', 'adding', 'failed', 'user_disabled'] as const
export type FinderSetup = (typeof FINDER_SETUPS)[number]

export type EngineState = 'running' | 'idle' | 'syncing' | 'paused' | 'offline' | 'error' | 'stopped'

/** `engine-status` event payload. The old keys are unchanged; the rest is new. */
export interface EngineStatusEvent {
  state: EngineState
  /** What the pre-slice-2 emitter would have said. Old consumers read this. */
  legacy_state?: string
  sync_root: string | null
  error: string | null
  files_remaining: number | null
  bytes_total: number | null
  bytes_done: number | null
  /** Unix seconds of the last check that reached the server; null = unknown. */
  last_tick_ok_at: number | null
  reason: { code: string; detail: string | null } | null
}

export type ActivityDirection = 'up' | 'down'
export type ActivityState = 'active' | 'done' | 'failed'

export interface ActivityRow {
  id: string
  name: string
  /** The immediate folder's name (`Investigations`), '' at the vault root. */
  folder: string
  /** Full path inside the vault, no leading slash (for a tooltip). */
  path: string
  /** Null only for a failed row. */
  direction: ActivityDirection | null
  state: ActivityState
  at: number
  size_bytes: number | null
  /** Null = unknown: show no percentage (spec state b'), never a guess. */
  done_bytes: number | null
  total_bytes: number | null
}

export interface PopoverSnapshot {
  phase: PopoverPhase
  generated_at: number
  account: { email: string | null; logged_in: boolean; vault_unlocked: boolean; auth_expired: boolean }
  paused: boolean
  engine: {
    state: string
    files_remaining: number | null
    bytes_total: number | null
    bytes_done: number | null
    last_tick_ok_at: number | null
  }
  reason: { code: string; detail: string | null } | null
  finder: { setup: FinderSetup; reason: string | null; reason_line: string | null }
  storage: { used_bytes: number; quota_bytes: number; fetched_at: number; stale: boolean } | null
  storage_full: boolean
  pending_changes: number
  conflicts: { count: number; files: { file_id: string; file_name: string }[] }
  activity: ActivityRow[]
}

// ── Validation ──────────────────────────────────────────────────────────────

type Obj = Record<string, unknown>

const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const isStr = (v: unknown): v is string => typeof v === 'string'
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)
const isBool = (v: unknown): v is boolean => typeof v === 'boolean'
const isNullOr = <T>(v: unknown, check: (x: unknown) => x is T): v is T | null => v === null || check(v)
const oneOf = <T extends string>(list: readonly T[], v: unknown): v is T => typeof v === 'string' && (list as readonly string[]).includes(v)

function isReason(v: unknown): boolean {
  return isObj(v) && isStr(v.code) && isNullOr(v.detail, isStr)
}

function isActivityRow(v: unknown): v is ActivityRow {
  return (
    isObj(v) &&
    isStr(v.id) &&
    isStr(v.name) &&
    isStr(v.folder) &&
    isStr(v.path) &&
    (v.direction === null || v.direction === 'up' || v.direction === 'down') &&
    oneOf(['active', 'done', 'failed'] as const, v.state) &&
    isNum(v.at) &&
    isNullOr(v.size_bytes, isNum) &&
    isNullOr(v.done_bytes, isNum) &&
    isNullOr(v.total_bytes, isNum)
  )
}

/** Narrow an `invoke('popover_snapshot')` result. Null when it is not the contract's shape. */
export function parsePopoverSnapshot(raw: unknown): PopoverSnapshot | null {
  if (!isObj(raw)) return null
  const { account, engine, finder, storage, conflicts, activity } = raw
  if (!oneOf(POPOVER_PHASES, raw.phase) || !isNum(raw.generated_at) || !isBool(raw.paused)) return null
  if (!isObj(account) || !isNullOr(account.email, isStr) || !isBool(account.logged_in) || !isBool(account.vault_unlocked) || !isBool(account.auth_expired)) return null
  if (
    !isObj(engine) ||
    !isStr(engine.state) ||
    !isNullOr(engine.files_remaining, isNum) ||
    !isNullOr(engine.bytes_total, isNum) ||
    !isNullOr(engine.bytes_done, isNum) ||
    !isNullOr(engine.last_tick_ok_at, isNum)
  ) return null
  if (raw.reason !== null && !isReason(raw.reason)) return null
  if (!isObj(finder) || !oneOf(FINDER_SETUPS, finder.setup) || !isNullOr(finder.reason, isStr) || !isNullOr(finder.reason_line, isStr)) return null
  if (
    storage !== null &&
    !(isObj(storage) && isNum(storage.used_bytes) && isNum(storage.quota_bytes) && isNum(storage.fetched_at) && isBool(storage.stale))
  ) return null
  if (!isBool(raw.storage_full) || !isNum(raw.pending_changes)) return null
  if (
    !isObj(conflicts) ||
    !isNum(conflicts.count) ||
    !Array.isArray(conflicts.files) ||
    !conflicts.files.every((f) => isObj(f) && isStr(f.file_id) && isStr(f.file_name))
  ) return null
  if (!Array.isArray(activity) || !activity.every(isActivityRow)) return null
  return raw as unknown as PopoverSnapshot
}

const ENGINE_STATES = ['running', 'idle', 'syncing', 'paused', 'offline', 'error', 'stopped'] as const

/** Narrow one `engine-status` event payload. Null when it is not the contract's shape. */
export function parseEngineStatusEvent(raw: unknown): EngineStatusEvent | null {
  if (!isObj(raw) || !oneOf(ENGINE_STATES, raw.state)) return null
  if (raw.legacy_state !== undefined && !isStr(raw.legacy_state)) return null
  if (!isNullOr(raw.sync_root, isStr) || !isNullOr(raw.error, isStr)) return null
  if (!isNullOr(raw.files_remaining, isNum) || !isNullOr(raw.bytes_total, isNum) || !isNullOr(raw.bytes_done, isNum)) return null
  if (!isNullOr(raw.last_tick_ok_at, isNum)) return null
  if (raw.reason !== null && !isReason(raw.reason)) return null
  return raw as unknown as EngineStatusEvent
}

// ── review:open ─────────────────────────────────────────────────────────────

export type ReviewMode = 'conflict' | 'versions'

/** Payload of `review:open`. camelCase, like the URL parameters it replaces. */
export interface ReviewOpen {
  fileId: string
  fileName: string
  isText: boolean
  mode: ReviewMode
}

/** A validated event, or null (a malformed event must never blank the window). */
export function parseReviewOpen(raw: unknown): ReviewOpen | null {
  if (!isObj(raw)) return null
  if (!isStr(raw.fileId) || raw.fileId.trim() === '') return null
  if (!isBool(raw.isText)) return null
  if (raw.mode !== 'conflict' && raw.mode !== 'versions') return null
  return {
    fileId: raw.fileId,
    fileName: isStr(raw.fileName) && raw.fileName !== '' ? raw.fileName : 'Unknown file',
    isText: raw.isText,
    mode: raw.mode,
  }
}

/** What the single `review` window is showing. `key` changes exactly when the window must remount. */
export interface ReviewTarget extends ReviewOpen {
  key: string
}

/**
 * The window's reducer: `ConflictWindow` reads its file from the URL once at mount,
 * so a window that is reused for another file remounts keyed by `key`. The key is
 * the file id and the mode: a repeated event for what is already shown keeps the
 * same target (no remount, no lost scroll position); a different file, or the same
 * file in the other mode, is a new mount. A malformed event keeps what is shown.
 */
export function nextReviewTarget(current: ReviewTarget | null, raw: unknown): ReviewTarget | null {
  const event = parseReviewOpen(raw)
  if (event === null) return current
  const key = `${event.fileId}:${event.mode}`
  if (current !== null && current.key === key) {
    // Same target. Take a corrected name or text flag without remounting.
    return current.fileName === event.fileName && current.isText === event.isText ? current : { ...event, key }
  }
  return { ...event, key }
}
