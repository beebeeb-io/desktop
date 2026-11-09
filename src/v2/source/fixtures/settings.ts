// The board's example values for B12's settings panes (Drive's Storage, Sync and File Explorer, Backups). Never in the
// running app (P1c): the dev server registers them through ../../board.ts (../../dev/board.ts) and answers the read
// commands with them (../../dev/reads.ts), so the frames and a plain browser draw the board's pages.
import { DEFAULT_CONFIG, type DesktopConfig, type StorageSummary } from '../../../desktopApi'
import type { KnownFolder } from '../../sections/settings/shell/data'
import type { SyncProblem } from '../extras'

const GB = 1024 ** 3

export const FIXTURE_CONFIG: DesktopConfig = { ...DEFAULT_CONFIG, sync_overlays: true, local_cache_limit_bytes: 50 * GB }

export const FIXTURE_FOLDERS: KnownFolder[] = [
  { key: 'desktop', display_name: 'Desktop', enabled: true, item_count: 412, size: '3.2 GB', last: '09:30' },
  { key: 'documents', display_name: 'Documents', enabled: true, item_count: 1820, size: '14.6 GB', last: '09:30' },
  { key: 'pictures', display_name: 'Pictures', enabled: true, item_count: 6208, size: '8.4 GB', last: '09:28' },
  { key: 'music', display_name: 'Music', enabled: false, item_count: 5344 },
  { key: 'videos', display_name: 'Videos', enabled: false, item_count: 88 },
  { key: 'downloads', display_name: 'Downloads', enabled: false, item_count: 301 },
  { key: 'projects', display_name: 'Projects', enabled: true, item_count: 905, size: '1.6 GB', last: '09:30', custom: true },
]

/** `desktop_storage_summary` as the board draws this PC: 18.4 GB downloaded, 1.4 GB Available offline (DW-0712). */
export const FIXTURE_LOCAL_SUMMARY: StorageSummary = { used_bytes: 240 * GB, quota_bytes: 1024 * GB, cache_bytes: 18.4 * GB, pinned_bytes: 1.4 * GB }

/** The board's preview cache (no command reads it yet): 3.1 GB of a 20 GB limit. */
export const FIXTURE_PREVIEWS = { bytes: 3.1 * GB, limitGb: 20, fits: '38,000' }

/** The board's other devices in Backups (DW-0737) and the device page (DW-0739). */
export const FIXTURE_BACKUP_DEVICES = [
  { id: 'iphone', name: 'Anna’s iPhone', status: 'Backed up 12,480 photos · 96 GB' },
  { id: 'ipad', name: 'Anna’s iPad', status: 'Backed up 4,120 photos · 38 GB' },
  { id: 'old', name: 'Old iPhone', status: 'Backed up 2,040 photos · 11 GB' },
]
export const FIXTURE_DEVICE_PAGE = { name: 'Anna’s iPad', backedUp: '4,120 photos · 38 GB', last: '19 Oct', status: 'Signed in' }

/** The board's status card (DW-0737, DW-1411). */
export const FIXTURE_BACKUP_CARD = {
  idle: { last: '09:30', bytes: '27.8 GB', failed: 3 },
  running: { folder: 'Documents', left: 'about 20 min left', done: 612, total: 1820 },
}

/** DW-0741's list (the board's): three failures with why, and one exclusion by design (no Try again). */
export const FIXTURE_BACKUP_PROBLEMS: SyncProblem[] = [
  { id: 'b-render', name: 'Render final.mov', folder: 'Documents', reason: 'the file is open in another app', retryable: true, backup: true },
  { id: 'b-invoice', name: 'Invoice scan.pdf', folder: 'Desktop', reason: 'this file is damaged', retryable: true, backup: true },
  { id: 'b-budget', name: 'Budget 2025.xlsx', folder: 'Documents', reason: 'it kept changing while it was backing up', retryable: true, backup: true },
  { id: 'b-site', name: 'Site walkthrough.mkv', folder: 'Desktop', reason: 'files larger than 4 GB are not backed up from this PC', retryable: false, backup: true },
]

/** The board's folder picker (DW-0738): Windows draws the real one. */
export const FIXTURE_PICKER_DIRS = ['Projects', 'Work', 'Archive', 'Scans']

/** The board's File Explorer state (DW-0746): Beebeeb is in File Explorer's navigation pane. */
export const FIXTURE_SHELL_STATE = { state: 'installed', path: 'C:\\Users\\Anna\\Beebeeb', error: null }

/** "Turn off" (DW-0746b): what File Explorer holds that hasn't uploaded. */
export const FIXTURE_UNSENT = { files: 2, bytes: 14 * 1024 ** 2 }
