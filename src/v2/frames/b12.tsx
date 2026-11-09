// Batch B12's frames (design/desktop-v2/BATCHES.md): the settings shell, Drive's sections and Backups on Windows.
import type { ReactElement } from 'react'
import { CaptionBar } from '../chrome/CaptionBar'
import { SettingsView } from '../sections/settings/shell/SettingsView'
import type { PaneId } from '../sections/settings/shell/context'
import { General } from '../sections/settings/shell/general'
import { Notifications } from '../sections/settings/shell/notifications'
import { Security } from './b12-security'
import { BackupFailures, Backups } from '../sections/settings/backups'
import { FIXTURE_BACKUP_PROBLEMS } from '../source/fixtures/settings'
import type { FrameRegistry } from './types'

/** The window with the settings view in it. `tall`: drawn taller than the window so the whole section shows. */
function win(pane: PaneId, opts: { dialog?: string; tall?: boolean; override?: ReactElement } = {}) {
  return () => (
    <div className={`flex ${opts.tall ? 'min-h-screen' : 'h-screen'} flex-col bg-paper font-sans text-ink`}>
      <CaptionBar title="Settings - Beebeeb" />
      <div className="flex min-h-0 flex-1 flex-col">
        <SettingsView pane={pane} dialog={opts.dialog ?? null} tall={opts.tall} override={opts.override} />
      </div>
    </div>
  )
}

export const FRAMES: FrameRegistry = {
  'DW-0701': win('general'),
  'DW-0781': win('general', { override: <General updateAvailable="0.9.2" /> }),
  'DW-0751': win('about', { tall: true }),
  'DW-0782': win('general', { dialog: 'report' }),
  'DW-0708': win('general', { dialog: 'plan' }),
  'DW-0712': win('storage'),
  'DW-0713': win('storage', { dialog: 'limit-previews' }),
  'DW-0714': win('storage', { dialog: 'limit-downloads' }),
  'DW-0780': win('storage', { dialog: 'free-up' }),
  'DW-0716': win('security', { override: <Security popupOpen /> }),
  'DW-0716b': win('security', { override: <Security lockOn={false} /> }),
  'DW-0750': win('notifications', { tall: true }),
  'DW-0750b': win('notifications', { tall: true, override: <Notifications offInWindows /> }),
  'DW-0746': win('sync'),
  'DW-0746b': win('sync', { dialog: 'explorer-off' }),
  'DW-0744': win('thumbnails', { tall: true }),
  'DW-0744b': win('thumbnails', { dialog: 'apply:2K:HEIC' }),
  'DW-0745': win('compare'),
  'DW-0737': win('backups'),
  'DW-1411': win('backups', { override: <Backups state="running" /> }),
  'DW-0738': win('backups', { dialog: 'picker' }),
  'DW-1410': win('backups', { dialog: 'stop:Projects' }),
  'DW-0739': win('device'),
  'DW-0742': win('device', { dialog: 'delete-backup' }),
  'DW-0741': win('backup-failures', { override: <BackupFailures problems={FIXTURE_BACKUP_PROBLEMS} /> }),
}
