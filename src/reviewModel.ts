/**
 * Review's pure decisions (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6 and §10.2
 * rows 11 to 17 and 35): the kind tags, the one action a row has, when and what a row says, the sidebar badge, the
 * conflict sheet's words and choices, and the version-history copy. Nothing here touches React or Tauri, so
 * `tests/reviewModel.test.ts` pins each decision; `ReviewPane.tsx`, `ConflictSheet.tsx` and `VersionHistorySheet.tsx`
 * only draw them.
 *
 * What stays in Rust: which entries exist, their kind, status and detail line (`version_conflict_feed_from_db`), and
 * what each conflict choice does to the files (`resolve_conflict`). This file does not classify anything again.
 */
import { actionNamedFor } from './actionName'
import { reviewEntryAction, type FileVersionEntry, type ReviewEntryActionKind, type ReviewEntryKind, type VersionConflictEntry } from './desktopApi'
import { formatStorageSize } from './storageFormat'
import { parseWhen, whenLabel } from './whenFormat'

export const REVIEW_INTRO = 'Files that need a decision from you, and changes Beebeeb couldn’t send yet.'
export const REVIEW_EMPTY = 'Nothing needs your attention.'
export const VERSIONS_LOAD_FAILED = 'Couldn’t load this file’s versions. Try again.'

// ── Rows ─────────────────────────────────────────────────────────────────────

/** §10.2 row 16: Conflict, Upload, Delete and Restore are unchanged; the other four are plain words. */
const KIND_LABELS: Record<ReviewEntryKind, string> = {
  conflict: 'Conflict',
  failed_upload: 'Upload',
  delete: 'Delete',
  restore: 'Restore',
  stale_base: 'Changed elsewhere',
  metadata: 'Rename or move',
  quota_failure: 'Storage full',
  permission_failure: 'No permission',
  // Not in the spec's table: the shipped list fell through to "Conflict" for it, which names the wrong thing on a row
  // whose button is "Sign in again". Flagged in the PR.
  auth_failure: 'Signed out',
  // Review of #142: a change that was parked, paused or given up and will not go on its own, and the plan refusing
  // uploads. Also not in the spec's table (flagged in the PR).
  not_sent: 'Not sent',
  account_refused: 'Plan',
  // A download or a pin that has not finished (and a kind of change the list has no other word for). Also what a kind this
  // build has never heard of is called.
  other: 'Other',
}

export function reviewKindLabel(kind: string): string {
  return (KIND_LABELS as Record<string, string | undefined>)[kind] ?? 'Other'
}

export interface ReviewRowAction {
  kind: ReviewEntryActionKind
  label: string
}

/** §10.2 row 17: "Review" is "Decide…" and "Upgrade" is "Add storage"; "Sign in again" is unchanged. */
const ACTION_LABELS: Record<ReviewEntryActionKind, string> = {
  open_conflict: 'Decide…',
  upgrade: 'Add storage',
  manage_plan: 'Manage plan',
  sign_in_again: 'Sign in again',
}

/**
 * The one button a row has, if any. The decision is `reviewEntryAction`'s (shared with the Versions page Linux keeps);
 * only the words differ on a Mac.
 */
export function reviewRowAction(entry: Pick<VersionConflictEntry, 'kind' | 'action'>): ReviewRowAction | null {
  const action = reviewEntryAction(entry)
  return action === null ? null : { kind: action.kind, label: ACTION_LABELS[action.kind] }
}

/**
 * What a screen reader calls a row's button. "Decide…" is about one file, so its name says which (a list of five
 * identical "Decide…" buttons tells nobody anything); the others act on the account, and their visible word is the name.
 * The visible word comes first, so the name contains it (WCAG 2.5.3).
 */
export function reviewActionName(entry: Pick<VersionConflictEntry, 'file_name'>, action: ReviewRowAction): string {
  return action.kind === 'open_conflict' ? actionNamedFor(action.label, entry.file_name) : action.label
}

/** "today 09:12", "yesterday 18:03", "12 Oct", "31 Dec 2025"; null when the engine has no time for the row. */
export function reviewWhen(updatedAt: number | null | undefined, now: Date): string | null {
  return whenLabel(updatedAt, now)
}

/**
 * A row's second line (§7.6, hi-fi section 3): the folder the file sits in, when it happened, then Rust's detail line:
 * "Photos · today 08:47 · Couldn’t upload, will retry". A file at the root of the vault has no folder. The detail lines
 * are plain sentences about what happened to the change (`engine_bridge.rs`, `REVIEW_DETAIL_*`; the failure's own text
 * is `last_error`, never shown).
 */
export function reviewRowMeta(entry: Pick<VersionConflictEntry, 'updated_at' | 'detail' | 'folder'>, now: Date): string {
  const when = reviewWhen(entry.updated_at, now)
  return [entry.folder?.trim(), when, entry.detail.trim()].filter((part): part is string => Boolean(part)).join(' · ')
}

const isText = (value: unknown): value is string => typeof value === 'string'
const isTextOrNothing = (value: unknown): boolean => value === undefined || value === null || typeof value === 'string'

/** One entry as Rust serialises it (`VersionConflictEntry`, `engine_bridge.rs`); anything else is no entry. */
export function parseReviewEntry(raw: unknown): VersionConflictEntry | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const e = raw as Record<string, unknown>
  const required = [e.id, e.file_id, e.file_name, e.kind, e.status, e.detail, e.action]
  if (!required.every(isText)) return null
  if (!(e.updated_at === undefined || e.updated_at === null || (typeof e.updated_at === 'number' && Number.isFinite(e.updated_at)))) return null
  if (!isTextOrNothing(e.folder) || !isTextOrNothing(e.op_id) || !isTextOrNothing(e.version_id) || !isTextOrNothing(e.last_error)) return null
  if (!(e.base_version === undefined || e.base_version === null || typeof e.base_version === 'number')) return null
  return e as unknown as VersionConflictEntry
}

/**
 * The feed as Rust sends it: an array, of which every entry that reads is kept and every one that does not is counted
 * (the pane says so: a Review that silently drops a row is a Review that says "Nothing needs your attention" about a
 * file that does). Not an array is no feed (null).
 */
export function parseReviewFeed(raw: unknown): { entries: VersionConflictEntry[]; rejected: number } | null {
  if (!Array.isArray(raw)) return null
  const entries = raw.map(parseReviewEntry).filter((entry): entry is VersionConflictEntry => entry !== null)
  return { entries, rejected: raw.length - entries.length }
}

/** The entries of [`parseReviewFeed`], for a reader that does not care what was refused. */
export function parseReviewEntries(raw: unknown): VersionConflictEntry[] | null {
  return parseReviewFeed(raw)?.entries ?? null
}

/** Shown under the rows when the answer carried entries this version of Beebeeb could not read. */
export const REVIEW_SOME_UNREADABLE = 'Some items couldn’t be shown.'

/**
 * The line under the rows when more wait than the feed carries (the engine caps it, and the sidebar's badge counts all
 * of them), or null when everything is shown.
 */
export function reviewMoreLine(count: number, shown: number): string | null {
  const more = count - shown
  if (!(more > 0)) return null
  return more === 1 ? '1 more is waiting. It appears here once you clear one of these.' : `${more} more are waiting. They appear here as you clear these.`
}

/** The number on Review's sidebar row: how many things wait, only while the vault is unlocked. */
export function reviewBadge(count: number, unlocked: boolean): number | null {
  return unlocked && count > 0 ? count : null
}

// ── The conflict sheet (§7.6, §10.2 row 35) ──────────────────────────────────

export type ConflictChoice = 'local' | 'remote' | 'both'

export const CONFLICT_SHEET_INTRO = 'This file was modified on two devices. Choose which version to keep.'
export const CONFLICT_SIDE_LABELS = { local: 'This Mac', remote: 'Other device' } as const

/** The three choices in the order of the hi-fi; "Keep both" is the safe one and the amber primary. */
export const CONFLICT_CHOICES: ReadonlyArray<{ choice: ConflictChoice; label: string; primary: boolean }> = [
  { choice: 'local', label: 'Keep this Mac’s', primary: false },
  { choice: 'remote', label: 'Keep the other', primary: false },
  { choice: 'both', label: 'Keep both', primary: true },
]

/**
 * "Keep the other" is the one choice that can leave this Mac with nothing of its own version: it downloads the other device's
 * file over the one the app keeps for this conflict. So it asks first, in words that claim only what the question is: "Use
 * the other version here?", with "Cancel" first (focused) and "Use the other version" as the one red button. Any other choice,
 * or closing the sheet, ends the question. The final copy is set after the device rung (task 2190): which file the choice
 * acts on, the app's own copy or the Finder item, is what that rung reads.
 */
export const CONFLICT_REPLACE_QUESTION = 'Use the other version here?'
export const CONFLICT_REPLACE_LABEL = 'Use the other version'
export const CONFLICT_REPLACE_CANCEL = 'Cancel'
/** The id of "Cancel" in the question, which takes the focus when it appears (the button that asked has left the footer). */
export const CONFLICT_REPLACE_CANCEL_ID = 'conflict-replace-cancel'
/** A press this soon after the question appeared is the same press (a double click, a held Return), not an answer. */
export const CONFLICT_REPLACE_GUARD_MS = 600

export const CONFLICT_RESOLVED_TOAST = { variant: 'success', message: 'Conflict resolved' } as const

export function conflictSheetTitle(fileName: string): string {
  return `Conflict: ${fileName}`
}

// ── Version history (§7.6, §10.2 rows 14 and 15) ─────────────────────────────

/** "Version 4", "Version 5 · current", or "Earlier version" when the server sent no number (never an id). */
export function versionLabel(version: FileVersionEntry): string {
  if (version.version_number == null) return version.is_current ? 'Current version' : 'Earlier version'
  return version.is_current ? `Version ${version.version_number} · current` : `Version ${version.version_number}`
}

/** "yesterday 08:00 · 48 KB", leaving out whatever the server did not say. */
export function versionLine(version: FileVersionEntry, now: Date, locale?: string): string {
  const when = parseWhen(version.created_at) === null ? null : whenLabel(version.created_at, now)
  const size = typeof version.size_bytes === 'number' ? formatStorageSize(version.size_bytes, locale) : null
  return [when, size].filter((part): part is string => Boolean(part)).join(' · ')
}

export function restoreQueuedToast(fileName: string, versionNumber: number | null | undefined) {
  const to = versionNumber == null ? 'an earlier version' : `version ${versionNumber}`
  return { variant: 'success', title: 'Restore queued', message: `${fileName} goes back to ${to} on the next sync.` } as const
}
