// B02b's fixtures: the board's numbers for the extras (../extras.ts), so every frame renders headless. Owned by B02b.
// `scenario` follows ?scenario=: `syncing` gives the flyout's moving state (DW-1203); `problems` the sync banner
// (DW-0102's "3 items can't sync") and the three problems Sync history lists; `listing` the getting-ready strip
// (DW-1107); `signed-out` the session that ended (DW-0105, DW-1109); `backup-problems` the backup banner (DW-0106);
// `conflict` one file changed in two places (DW-0429); `uploading` the uploads in Lisbon trip (DW-0426, DW-0427);
// `default` is up to date, with no problems and nothing moving (DW-0401).
import { DriveSourceError, type DriveItem } from '@beebeeb/shared/drive-ui'
import { command } from '../../../desktopApi'
import type { DriveExtras, DriveStatus, PhotoMeta, SyncProblem, TransferRow } from '../extras'
import { HISTORY, OFFLINE_ENTRIES, PHOTO_ENTRIES, PHOTO_META } from './photos'
import { SEARCH_ITEMS } from './shared'

/** Where each of B05's search hits is (DW-0802: "Documents › Legal"); anything else is in Files. */
const WHERE: Record<string, string> = Object.fromEntries(SEARCH_ITEMS.map((r) => [r.item.id, r.where]))

const MB = 1_000_000

/** DW-0427: "3 of 5 · 42 MB left". */
export const FIXTURE_TRANSFERS: TransferRow[] = [
  { id: 'u1', name: 'Scan 12 Oct.pdf', folder: 'Documents', direction: 'up', state: 'stopped', bytes: 0, total: 900_000, reason: 'Stopped after 5 tries. Not enough space.', retryable: true },
  { id: 'p-miradouro', fileId: 'p-miradouro', name: 'Miradouro.heic', folder: 'Lisbon trip', direction: 'up', state: 'moving', bytes: 2_100_000, total: 3_600_000, retryable: false },
  { id: 'u3', fileId: 'u3', name: 'Tram 28.mov', folder: 'Lisbon trip', direction: 'up', state: 'moving', bytes: 34 * MB, total: 82 * MB, retryable: false },
  { id: 'p-ponte', fileId: 'p-ponte', name: 'Ponte.heic', folder: 'Lisbon trip', direction: 'up', state: 'waiting', bytes: 0, total: 3_200_000, retryable: false },
]

/** DW-0428's three problems, from B06's history rows. */
export const FIXTURE_PROBLEMS: SyncProblem[] = HISTORY.filter((r) => r.problem).map((r) => ({
  id: r.id, fileId: r.itemId, name: r.name, folder: r.what.replace(/^Didn’t sync · /, ''), reason: r.problem!, retryable: true, backup: false, at: r.at,
}))

export function fixtureStatus(scenario: string): DriveStatus {
  return {
    signedOut: false,
    stoppedFiles: 0,
    syncProblems: 0,
    backupProblems: 0,
    fileCount: 1_284,
    freeSpaceBytes: 12.4 * 1024 ** 3, // the board's 12.4 GB (DW-1517), in the units fmtBytes shows
    backup: { state: 'on', lastBackupAt: '2026-11-08T11:52:00', deviceFolder: 'Anna-Laptop', foldersOn: 3, foldersGettingReady: 0 },
    conflicts: [],
    ...(scenario === 'syncing' ? { uploadBps: 1_400_000, etaSeconds: 60 } : {}),
    ...(scenario === 'problems' ? { syncProblems: 3, stoppedFiles: 1 } : {}),
    ...(scenario === 'listing' ? { listing: { done: 18_240, total: 40_112 } } : {}),
    ...(scenario === 'signed-out' ? { signedOut: true } : {}),
    ...(scenario === 'backup-problems' ? { backupProblems: 3 } : {}),
    ...(scenario === 'conflict' ? { conflicts: [{ id: 'i-contract', name: 'Contract draft.pdf' }] } : {}),
  }
}

function photoMeta(id: string): PhotoMeta | undefined {
  const m = PHOTO_META[id]
  if (!m) return undefined
  const [w, h] = (m.dims ?? '').split(' × ').map(Number)
  const [min, sec] = (m.duration ?? '').split(':').map(Number)
  return {
    takenAt: PHOTO_ENTRIES.find((e) => e.item.id === id)?.takenAt,
    camera: m.camera, width: w || undefined, height: h || undefined,
    durationSeconds: m.duration ? min * 60 + sec : undefined, uploading: !!m.uploading, device: m.device,
  }
}

export function createFixtureExtras(scenario: string): DriveExtras {
  // `empty` (DW-0114, DW-1305, DW-0120): a new account, nothing moving, nothing kept, no problems.
  const empty = scenario === 'empty'
  return {
    status: async () => fixtureStatus(scenario),
    // Uploads move only where the board shows them (DW-0426, DW-0427; the flyout's syncing): an up-to-date world has none.
    transfers: async () => (scenario === 'uploading' || scenario === 'syncing' ? FIXTURE_TRANSFERS : []),
    subscribeTransfers: () => () => {},
    // The list agrees with the status: problems only where drive_status counts them (DW-0401 has none).
    syncProblems: async () => (scenario === 'problems' ? FIXTURE_PROBLEMS : []),
    retry: async (ids) => ({ retried: ids.length }),
    offlineList: async () => (empty ? [] : OFFLINE_ENTRIES).map((o) => ({ item: o.item, items: o.item.isFolder ? o.item.childCount : undefined, bytes: o.bytes ?? 0 })),
    photoMeta: async (ids) => Object.fromEntries(ids.flatMap((id) => { const m = photoMeta(id); return m ? [[id, m]] : [] })),
    // DW-0604: the backups' folders (the devices) and the board's folder from Files, Lisbon trip.
    photoSources: async () => (empty ? [] : [
      { id: 'f-backups', name: 'Camera uploads', path: 'Backups/Anna’s iPhone/Camera uploads', count: PHOTO_ENTRIES.length },
      { id: 'f-lisbon', name: 'Lisbon trip', path: 'Lisbon trip', count: 212 },
    ]),
    paths: async (ids) => Object.fromEntries(ids.map((id) => [id, WHERE[id] ?? 'Files'])),
    // A write: it goes to the host as the desktop side sends it. A plain browser's dev host has no such command, so it
    // is "unavailable" (the map audit's command-unsupported), never a made-up file (M3d).
    async createFile(parentId, name) {
      const r = await command<DriveItem>('drive_create_file', { parentId, name })
      if (r.ok) return r.value
      throw new DriveSourceError(r.unsupported ? 'unsupported' : 'failed', 'drive_create_file')
    },
  }
}
