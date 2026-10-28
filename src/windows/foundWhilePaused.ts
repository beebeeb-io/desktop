/**
 * Changes found while sync was paused (task 2267, Windows): the words of the Settings › Sync card, its result lines and
 * when it shows. Pure, so it is tested without React (`tests/foundWhilePaused.test.ts`); the card is
 * `FoundWhilePausedCard.tsx`.
 *
 * When Beebeeb adopts a PC's local data after proving it belongs to the signed-in account, local items it could NOT
 * prove (new top-level folders and files made while sync was refused, or changes inside shared-with-me folders) are not
 * uploaded on their own. The backend holds their queued operations with the pause reason `found_while_paused`, and the
 * person decides: `found_while_paused_upload` releases them, `found_while_paused_keep` moves the local-only ones out of
 * the sync folder into `<parent of the sync folder>\Beebeeb - kept on this PC\YYYY-MM-DD\` and stops tracking them.
 *
 * Copy rules (as `syncActivityModel.ts`): counts only, never a file name; the one path shown is the folder the backend
 * reports it moved them to, which the person needs to find them.
 */
import { plural, type SyncActivitySummary } from './syncActivityModel'

/** The `HeldGroup.reason` the backend gives these changes. */
export const FOUND_WHILE_PAUSED = 'found_while_paused'

/** The folder the backend creates next to the sync folder (its dated subfolder is `kept_path`). */
export const KEPT_FOLDER_NAME = 'Beebeeb - kept on this PC'

export const FOUND_WHILE_PAUSED_COPY = {
  title: 'Files Beebeeb found while sync was paused. They haven’t been uploaded yet.',
  upload: 'Upload to my Beebeeb',
  keep: 'Keep on this PC only',
  confirm: `They’ll be moved out of your Beebeeb folder into a folder named “${KEPT_FOLDER_NAME}” next to it. Nothing is uploaded or deleted.`,
  move: 'Move them',
  cancel: 'Cancel',
  showInExplorer: 'Show in Explorer',
  dismiss: 'Dismiss',
} as const

/** `found_while_paused_keep`, field for field. */
export interface KeepResult {
  /** Files moved, counted inside folders too. */
  moved_files: number
  /** Top-level items moved. */
  moved_items: number
  /** The dated folder they were moved into; null when nothing moved. */
  kept_path: string | null
  /** Items left in place and still held (on another drive, or the move failed). */
  not_moved: number
  /** Held changes no move can keep off Beebeeb (the item is already there, or its upload had begun): they stay held. */
  stays_held: number
}

const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0

/** Narrow a `found_while_paused_keep` answer. Null when it is not the contract's shape: the card then shows an error. */
export function parseKeepResult(raw: unknown): KeepResult | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const r = raw as Record<string, unknown>
  if (!isCount(r.moved_files) || !isCount(r.moved_items) || !isCount(r.not_moved) || !isCount(r.stays_held)) return null
  if (!(r.kept_path === null || typeof r.kept_path === 'string')) return null
  return { moved_files: r.moved_files, moved_items: r.moved_items, kept_path: r.kept_path, not_moved: r.not_moved, stays_held: r.stays_held }
}

/** The changes held as found while sync was paused, from the last summary read (0 when it was not read). */
export function foundWhilePausedCount(summary: SyncActivitySummary | null | undefined): number {
  return (summary?.queue?.held ?? [])
    .filter((group) => group.reason === FOUND_WHILE_PAUSED)
    .reduce((sum, group) => sum + group.count, 0)
}

export function waitingForDecisionLine(n: number): string {
  return `${plural(n, 'change', 'changes')} waiting for your decision.`
}

export interface KeepResultLine {
  key: 'moved' | 'not_moved' | 'stays_held'
  /** The whole sentence. */
  text: string
  /** For `moved`: the sentence split around the folder path, so the path can be drawn in mono. */
  path?: { before: string; path: string; after: string }
}

/**
 * What a keep did, one line per count that is not zero: moved, then left where it was, then still held. Nothing to
 * say is no line (the card then shows no result). When only empty folders moved, the line counts items: "Moved 0
 * files" would read as if nothing happened while the folder exists.
 */
export function keepResultLines(result: KeepResult): KeepResultLine[] {
  const lines: KeepResultLine[] = []
  if (result.moved_files > 0 || result.moved_items > 0) {
    const counted = result.moved_files > 0
      ? { n: result.moved_files, what: plural(result.moved_files, 'file', 'files') }
      : { n: result.moved_items, what: plural(result.moved_items, 'item', 'items') }
    const before = `Moved ${counted.what} to `
    const path = result.kept_path ?? KEPT_FOLDER_NAME
    const after = counted.n === 1 ? '. It stays on this PC and isn’t uploaded.' : '. They stay on this PC and aren’t uploaded.'
    lines.push({ key: 'moved', text: `${before}${path}${after}`, path: { before, path, after } })
  }
  if (result.not_moved > 0) {
    const n = result.not_moved
    lines.push({
      key: 'not_moved',
      text: n === 1
        ? '1 item couldn’t be moved and stays where it is. It isn’t uploaded.'
        : `${plural(n, 'item', 'items')} couldn’t be moved and stay where they are. They aren’t uploaded.`,
    })
  }
  if (result.stays_held > 0) {
    const n = result.stays_held
    lines.push({
      key: 'stays_held',
      text: n === 1
        ? `1 change can’t be kept on this PC only. It isn’t uploaded and stays on hold until you choose “${FOUND_WHILE_PAUSED_COPY.upload}”.`
        : `${plural(n, 'change', 'changes')} can’t be kept on this PC only. They aren’t uploaded and stay on hold until you choose “${FOUND_WHILE_PAUSED_COPY.upload}”.`,
    })
  }
  return lines
}

/**
 * The card shows while changes wait for the decision, while one of its commands runs (a poll that lands mid-move must
 * not drop the result), and while a keep result with something to say is on screen (until dismissed).
 */
export function foundWhilePausedCardVisible(
  summary: SyncActivitySummary | null | undefined,
  state: { busy: boolean; kept: KeepResult | null },
): boolean {
  if (foundWhilePausedCount(summary) > 0 || state.busy) return true
  return state.kept !== null && keepResultLines(state.kept).length > 0
}
