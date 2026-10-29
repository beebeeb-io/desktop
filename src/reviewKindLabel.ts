import type { VersionConflictEntry } from './desktopApi'

/**
 * The kind tag a Versions row shows. Words come from the macOS spec's copy table
 * (docs/specs/2026-10-24-macos-two-surface-app.md, row 16), not from engine names.
 */
export function reviewKindLabel(kind: VersionConflictEntry['kind']): string {
  switch (kind) {
    case 'quota_failure':
      return 'Storage full'
    case 'permission_failure':
      return 'No permission'
    case 'stale_base':
      return 'Changed elsewhere'
    case 'failed_upload':
      return 'Upload'
    case 'metadata':
      return 'Rename or move'
    case 'delete':
      return 'Delete'
    case 'restore':
      return 'Restore'
    case 'kept_copy':
      return 'Kept copy'
    case 'held_edit':
    case 'held_unknown':
      return 'Not sent'
    // Review-169 #6 (found by its kind census): an expired session's row had fallen to "Conflict".
    // Its action stays "Sign in again" (reviewEntryAction); the tag names the state (Guus, 2026-10-28).
    case 'auth_failure':
      return 'Signed out'
    default:
      return 'Conflict'
  }
}
