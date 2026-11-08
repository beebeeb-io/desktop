// Pane and dialog tables for B12.
import type { ComponentType } from 'react'
import type { PaneId } from './context'
import { General, About, ReportDialog, PlanDialog, RegionDialog } from './general'
import { Notifications } from './notifications'
import { Security } from './security'
import { Storage, PreviewLimitDialog, DownloadLimitDialog, FreeUpDialog, InsightsDialog } from '../drive/storage'
import { SyncPane, ExplorerOffDialog, RepairDialog } from '../drive/sync'
import { Thumbnails, Compare } from '../drive/thumbnails'
import { Backups, BackupFailures, DevicePane, StopBackupDialog, DeleteBackupDialog, FolderPicker } from '../backups'

export const BASE_PANES: Partial<Record<PaneId, ComponentType>> = {
  general: General, about: About, notifications: Notifications, security: Security,
  storage: Storage, sync: SyncPane, thumbnails: Thumbnails, compare: Compare,
  backups: Backups, 'backup-failures': BackupFailures, device: DevicePane,
}

export const DIALOGS: Record<string, ComponentType<{ arg?: string }>> = {
  report: ReportDialog, plan: PlanDialog, region: RegionDialog,
  'limit-previews': PreviewLimitDialog, 'limit-downloads': DownloadLimitDialog, 'free-up': FreeUpDialog, insights: InsightsDialog,
  'explorer-off': ExplorerOffDialog, repair: RepairDialog,
  // `apply` is drawn by the Thumbnails pane itself (it owns the choice it applies); this entry covers a direct open.
  apply: () => null,
  stop: StopBackupDialog, 'delete-backup': DeleteBackupDialog, picker: FolderPicker,
}
