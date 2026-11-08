// B02b's fixtures: the board's numbers for the extras (../extras.ts), so every frame renders headless. Owned by B02b.
// `scenario` follows ?scenario=: `syncing` gives the flyout's moving state (DW-1203); `problems` the sync banner
// (DW-0103's "3 items can't sync"); `listing` the getting-ready strip (DW-1107); `default` is up to date.
import type { DriveExtras, DriveStatus, PhotoMeta, SyncProblem, TransferRow } from '../extras'
import { HISTORY, OFFLINE_ENTRIES, PHOTO_ENTRIES, PHOTO_META } from './photos'

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
    freeSpaceBytes: 12_400_000_000,
    backup: { state: 'on', lastBackupAt: '2026-11-08T11:52:00', deviceFolder: 'Anna-Laptop', foldersOn: 3, foldersGettingReady: 0 },
    conflicts: [],
    ...(scenario === 'syncing' ? { uploadBps: 1_400_000, etaSeconds: 60 } : {}),
    ...(scenario === 'problems' ? { syncProblems: 3, stoppedFiles: 1 } : {}),
    ...(scenario === 'listing' ? { listing: { done: 18_240, total: 40_112 } } : {}),
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
    durationSeconds: m.duration ? min * 60 + sec : undefined, uploading: !!m.uploading,
  }
}

export function createFixtureExtras(scenario: string): DriveExtras {
  return {
    status: async () => fixtureStatus(scenario),
    transfers: async () => FIXTURE_TRANSFERS,
    subscribeTransfers: () => () => {},
    syncProblems: async () => FIXTURE_PROBLEMS,
    retry: async (ids) => ({ retried: ids.length }),
    offlineList: async () => OFFLINE_ENTRIES.map((o) => ({ item: o.item, items: o.item.isFolder ? o.item.childCount : undefined, bytes: o.bytes ?? 0 })),
    photoMeta: async (ids) => Object.fromEntries(ids.flatMap((id) => { const m = photoMeta(id); return m ? [[id, m]] : [] })),
    photoSources: async () => [{ id: 'f-backups', name: 'Camera uploads', path: 'Backups/Anna’s iPhone/Camera uploads', count: PHOTO_ENTRIES.length }],
    paths: async (ids) => Object.fromEntries(ids.map((id) => [id, 'Files'])),
  }
}
