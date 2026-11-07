// LD-0139 (spec Amendments 12 and 12c): why a backup stopped, in core's words, sent with each folder by
// `get_known_folder_backup` (Rust `backup::stop::StopView`). The words are core's (`backup_stop_words`,
// `MSG_BACK_UP_AGAIN`, `MSG_STOP_CHECK_FAILED`); this file and `backupStatus` (WindowsApp/filesView.tsx) only
// place them.

/** The stop fields `get_known_folder_backup` flattens into each folder. */
export interface BackupStopFields {
  enabled: boolean
  /** Why this backup is off, when it turned itself off (null when on, or switched off by the person). */
  stop_words?: string | null
  /** The action under the words: "Back up again". */
  stop_action?: string | null
  /** "Couldn't check — will retry." on a backup that is on, while the last notices check failed. */
  check_note?: string | null
}

/** The label of the button that starts a stopped backup again, or null when it did not stop by itself. */
export function backUpAgainLabel(f: BackupStopFields): string | null {
  return !f.enabled && f.stop_words && f.stop_action ? f.stop_action : null
}
