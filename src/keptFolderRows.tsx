/**
 * The two "files were kept" rows (task 1882 and task 1887), one hook and one component for every place that shows
 * them. Task 1683 slice 1 (two-surface spec §8.3): Settings › Sync keeps them where both specs put them, and Home's
 * "Needs your attention" shows the same component with the same words and buttons, reading the same
 * `kept_unsent_folder` / `kept_unsynced_folder` state, so a dismiss in either place clears both. The code moved out of
 * `SyncTab` unchanged; its tests moved with it.
 *
 * - 1887 (spec 2026-10-10 §9): the folder a sign-out copied unsent changes into, with "Show in Finder" and "Dismiss".
 *   Ours comes first.
 * - 1882: where macOS last kept Finder files that had not reached the server (a sign-out, a Repair, another reconciler
 *   removal, the app-start sweep), with "Dismiss".
 *
 * Rust saves either folder in desktop.toml and emits `kept-folder-changed` when it does; each row reads its folder
 * again then. The first read waits for the listener, so a folder saved between the two is not missed. A failed read
 * shows no row: the folder stays saved, and the next open shows it again.
 */
import { useCallback, useEffect, useState } from 'react'
import { command, commandUnavailableLabel } from './desktopApi'
import { subscribeKeptFolderChanged } from './finderSetup'
import { KEPT_FOLDER_ROW_SENTENCE, keptFolderAfterDismiss, preservedFilesNote, type DismissKeptFolderResult } from './macSettingsModel'
import { Btn, Note } from './macSettingsParts'
import { K2_DISMISS, K2_ROW, K2_SHOW, K3_TOAST } from './signOutCopy'
import { useToast } from './windows/ui'

export interface KeptFolders {
  /** 1882's folder, or null. */
  kept: string | null
  /** Repair reports the folder it kept; it shows that one when no other is saved (see `SyncTab`'s `runRepair`). */
  setKept: (next: string | null | ((current: string | null) => string | null)) => void
  loadKept: () => Promise<void>
  /** 1887's folder, or null. */
  keptUnsent: string | null
  showKeptUnsent: () => Promise<void>
  dismissKeptUnsent: () => Promise<void>
  dismissKept: () => Promise<void>
}

export function useKeptFolders(): KeptFolders {
  const { showToast } = useToast()
  // Task 1882 round 2 (review I2, lead ruling): shown until the person dismisses it, so a section switch, closing the
  // window or a restart cannot lose it.
  const [kept, setKept] = useState<string | null>(null)

  const loadKept = useCallback(async () => {
    const result = await command<string | null>('kept_unsynced_folder')
    const note = result.ok ? preservedFilesNote({ preserved_location: result.value }) : null
    setKept(note ? note.path : null)
  }, [])

  // Rebase re-review I2: a removal the reconciler runs in the background saves the folder long after this row opened.
  useEffect(
    () => subscribeKeptFolderChanged(() => void loadKept(), { onSubscribed: () => void loadKept() }),
    [loadKept],
  )

  // Spec 2026-10-10 §9: Rust saves it before the purge and emits `kept-folder-changed`.
  const [keptUnsent, setKeptUnsent] = useState<string | null>(null)
  const loadKeptUnsent = useCallback(async () => {
    const result = await command<string | null>('kept_unsent_folder')
    setKeptUnsent(result.ok ? result.value : null)
  }, [])
  useEffect(
    () => subscribeKeptFolderChanged(() => void loadKeptUnsent(), { onSubscribed: () => void loadKeptUnsent() }),
    [loadKeptUnsent],
  )

  // "Show in Finder" fails only when the folder is gone (K3); the row stays until Dismiss.
  const showKeptUnsent = async () => {
    if (keptUnsent === null) return
    const result = await command<void>('show_kept_unsent_folder', { path: keptUnsent })
    if (!result.ok) showToast({ variant: 'error', message: K3_TOAST })
  }
  const dismissKeptUnsent = async () => {
    if (keptUnsent === null) return
    const result = await command<DismissKeptFolderResult>('dismiss_kept_unsent_folder', { path: keptUnsent })
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t dismiss that note',
        message: result.unsupported ? commandUnavailableLabel('dismiss_kept_unsent_folder') : result.reason,
      })
      return
    }
    setKeptUnsent(keptFolderAfterDismiss(result.value))
  }

  // A one-off action that gates nothing: a failure is a toast (house rule), and the row stays.
  const dismissKept = async () => {
    if (kept === null) return
    const result = await command<DismissKeptFolderResult>('dismiss_kept_unsynced_folder', { path: kept })
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t dismiss that note',
        message: result.unsupported ? commandUnavailableLabel('dismiss_kept_unsynced_folder') : result.reason,
      })
      return
    }
    // 1882 r5: the row goes only if Rust cleared the folder it showed. If a newer folder was kept since the row was
    // drawn, nothing was dismissed, and the row shows the newer one.
    setKept(keptFolderAfterDismiss(result.value))
  }

  return { kept, setKept, loadKept, keptUnsent, showKeptUnsent, dismissKeptUnsent, dismissKept }
}

/** The rows themselves: ours first, then 1882's. Nothing when neither folder is saved. */
export function KeptFolderNotes({ folders }: { folders: KeptFolders }) {
  return (
    <>
      {folders.keptUnsent !== null ? (
        <Note
          kind="status"
          reason={folders.keptUnsent}
          reasonWraps
          actions={
            <>
              <Btn onClick={() => void folders.showKeptUnsent()}>{K2_SHOW}</Btn>
              <Btn onClick={() => void folders.dismissKeptUnsent()}>{K2_DISMISS}</Btn>
            </>
          }
        >
          {K2_ROW}
        </Note>
      ) : null}
      {folders.kept !== null ? (
        <Note
          kind="status"
          reason={folders.kept}
          reasonWraps
          actions={<Btn onClick={() => void folders.dismissKept()}>Dismiss</Btn>}
        >
          {KEPT_FOLDER_ROW_SENTENCE}
        </Note>
      ) : null}
    </>
  )
}
