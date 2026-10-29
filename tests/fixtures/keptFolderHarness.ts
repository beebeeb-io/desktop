/**
 * The kept-folder rows as the component harness mounts them (task 1683 slice 1): the REAL `useKeptFolders` declaration
 * (run with the mounted component's own hooks, like `useFinderSetup`) and the REAL `KeptFolderNotes` component, both
 * from `src/keptFolderRows.tsx`. Settings › Sync and Home use the same two, so their tests bind the same two.
 * `overrides` replaces what the hook reads, e.g. a scripted `subscribeKeptFolderChanged`.
 */
import { createElement, Fragment } from 'react'
import * as desktopApi from '../../src/desktopApi'
import * as finderSetup from '../../src/finderSetup'
import * as model from '../../src/macSettingsModel'
import * as parts from '../../src/macSettingsParts'
import * as signOutCopy from '../../src/signOutCopy'
import { loadComponent, type HookModule } from './componentHarness'

export function useKeptFoldersModule(overrides: Record<string, unknown> = {}): HookModule {
  return {
    file: 'keptFolderRows.tsx',
    name: 'useKeptFolders',
    bindings: {
      command: desktopApi.command,
      commandUnavailableLabel: desktopApi.commandUnavailableLabel,
      subscribeKeptFolderChanged: finderSetup.subscribeKeptFolderChanged,
      preservedFilesNote: model.preservedFilesNote,
      keptFolderAfterDismiss: model.keptFolderAfterDismiss,
      K3_TOAST: signOutCopy.K3_TOAST,
      ...overrides,
    },
  }
}

export const KeptFolderNotes = loadComponent('keptFolderRows.tsx', 'KeptFolderNotes', {
  React: { createElement, Fragment },
  Note: parts.Note,
  Btn: parts.Btn,
  KEPT_FOLDER_ROW_SENTENCE: model.KEPT_FOLDER_ROW_SENTENCE,
  K2_ROW: signOutCopy.K2_ROW,
  K2_SHOW: signOutCopy.K2_SHOW,
  K2_DISMISS: signOutCopy.K2_DISMISS,
})
