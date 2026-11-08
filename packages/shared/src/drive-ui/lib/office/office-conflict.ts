/**
 * Save-conflict decisions for the office editor (task 1567) — the binary-file
 * sibling of `../editor-conflict.ts`.
 *
 * Same write-ahead-conflict-check shape as the text editor (task 1563):
 * `hasVersionConflict` is reused unchanged from that module. The action set
 * differs because a diff view makes no sense for a binary .docx/.xlsx/.pptx —
 * per the task brief, "Show differences" is replaced by "Discard" (throw
 * away this session's local edit and reopen the server's current version).
 *
 * Pure — no DOM, no network — unit-testable in isolation.
 */

export { hasVersionConflict, insertBeforeExtension, type ConflictCheck } from '../editor-conflict'

export type OfficeConflictAction = 'keep-both' | 'save-as-new-version' | 'discard'

export interface OfficeConflictResolution {
  /** file_id to target. Same file id => the server appends a new version.
   *  undefined => a brand-new sibling file (Keep Both). */
  fileId: string | undefined
  conflictCreated: boolean
  nameSuffix?: string
}

/**
 * Resolves a chosen conflict action into upload parameters, or null when
 * there is nothing to upload this call ('discard' — the caller throws away
 * the local edit and reopens the server's current bytes instead).
 */
export function resolveOfficeConflictAction(
  action: OfficeConflictAction,
  fileId: string,
): OfficeConflictResolution | null {
  switch (action) {
    case 'save-as-new-version':
      return { fileId, conflictCreated: false }
    case 'keep-both':
      return { fileId: undefined, conflictCreated: true, nameSuffix: ' (edited on web)' }
    case 'discard':
      return null
  }
}
