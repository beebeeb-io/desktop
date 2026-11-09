// Pane and dialog tables for B12. About, Security, Sync and File Explorer and the repair alert are the later batches'
// (panes-today.ts, panes-account.ts); B12's first drawings of them, with the board's numbers, are gone from the app
// (P1c: B12's Security is a frame's drawing, frames/b12-security.tsx).
import type { ComponentType } from 'react'
import type { PaneId } from './context'
import { General, ReportDialog, PlanDialog, RegionDialog } from './general'
import { Notifications } from './notifications'
import { Storage, PreviewLimitDialog, DownloadLimitDialog, FreeUpDialog } from '../drive/storage'
import { ExplorerOffDialog } from '../drive/sync'
import { Thumbnails, Compare } from '../drive/thumbnails'
import { Backups, BackupFailures, DevicePane, StopBackupDialog, DeleteBackupDialog, FolderPicker } from '../backups'

export const BASE_PANES: Partial<Record<PaneId, ComponentType>> = {
  general: General, notifications: Notifications,
  storage: Storage, thumbnails: Thumbnails, compare: Compare,
  backups: Backups, 'backup-failures': BackupFailures, device: DevicePane,
}

export const DIALOGS: Record<string, ComponentType<{ arg?: string }>> = {
  report: ReportDialog, plan: PlanDialog, region: RegionDialog,
  'limit-previews': PreviewLimitDialog, 'limit-downloads': DownloadLimitDialog, 'free-up': FreeUpDialog,
  'explorer-off': ExplorerOffDialog,
  // `apply` is drawn by the Thumbnails pane itself (it owns the choice it applies); this entry covers a direct open.
  apply: () => null,
  stop: StopBackupDialog, 'delete-backup': DeleteBackupDialog, picker: FolderPicker,
}
