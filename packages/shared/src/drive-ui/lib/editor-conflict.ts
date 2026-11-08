/**
 * Save-conflict decision logic for the in-browser text editor (task 1563).
 *
 * The editor snapshots the file's version number when it opens. Before it
 * uploads a save, it re-checks the server's current version number. If that
 * number has moved on (someone else — another device, another tab — saved
 * first), the save is NOT sent silently; the caller shows the conflict
 * dialog (design/editor-1563.html screen 04) and the user picks one of three
 * actions. This module is the pure decision core: given the opened/current
 * version numbers and the chosen action, what should the caller do next.
 *
 * No DOM, no network, no crypto — unit-testable in isolation.
 */

export interface ConflictCheck {
  /** The version number this edit session started from. */
  openedVersion: number
  /** The version number the server reports right before saving. */
  serverVersion: number
}

/**
 * True when another save landed after this edit session opened the file.
 * Equal or lower server versions are not a conflict — lower can happen if a
 * version was deleted/restored, in which case there's nothing to protect
 * against overwriting that this session hasn't already seen.
 */
export function hasVersionConflict({ openedVersion, serverVersion }: ConflictCheck): boolean {
  return serverVersion > openedVersion
}

export type ConflictAction = 'keep-both' | 'save-as-new-version' | 'show-differences'

export interface ConflictResolution {
  /** file_id to target. Same file id => the server appends a new version.
   *  undefined => a brand new file (Keep Both). */
  fileId: string | undefined
  /** Passed through to initUpload's conflict_created flag. */
  conflictCreated: boolean
  /** Suffix appended before the extension for the Keep Both copy. */
  nameSuffix?: string
}

/**
 * Resolves a chosen conflict action into upload parameters.
 *
 * - 'save-as-new-version': proceed with the SAME file id. The edit becomes
 *   the next version; the version that raced it (e.g. version 5) is left
 *   untouched in history — nothing is overwritten, per the design's "no
 *   silent overwrite, ever".
 * - 'keep-both': upload as a NEW file (fresh id, caller generates it) named
 *   "<original> (edited on web).<ext>", with conflict_created so the server
 *   applies the same accounting it uses for the drag-drop Keep Both path.
 * - 'show-differences': no upload yet — the caller renders the diff view
 *   and waits for a follow-up action, so this returns null.
 */
export function resolveConflictAction(
  action: ConflictAction,
  fileId: string,
): ConflictResolution | null {
  switch (action) {
    case 'save-as-new-version':
      return { fileId, conflictCreated: false }
    case 'keep-both':
      return { fileId: undefined, conflictCreated: true, nameSuffix: ' (edited on web)' }
    case 'show-differences':
      return null
  }
}

/**
 * Inserts a suffix before a filename's extension: "notes.md" + " (edited on
 * web)" => "notes (edited on web).md". A name with no extension gets the
 * suffix appended at the end.
 */
export function insertBeforeExtension(filename: string, suffix: string): string {
  const dot = filename.lastIndexOf('.')
  if (dot <= 0) return `${filename}${suffix}`
  return `${filename.slice(0, dot)}${suffix}${filename.slice(dot)}`
}
