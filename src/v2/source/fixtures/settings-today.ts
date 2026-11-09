// Fixture data for B13's settings panes (the board's example numbers). A plain browser has no IPC bridge, so these
// stand in; with the bridge the panes read through command().
import { GB, MB } from '../../sections/settings/shell/kit'
import { failureSentence, type SpeedResult, type TestState } from '../../sections/settings/today/speedtest'

export type BwVariant = 'normal' | 'idle' | 'limited' | 'empty' | 'metered'

/** 60 buckets of 20 minutes, oldest first: 13:40 yesterday to now. Values in MB moved per bucket. */
export function chartBuckets(v: BwVariant): { up: number; down: number }[] {
  if (v === 'empty') return []
  return Array.from({ length: 60 }, (_, i) => {
    const wave = (k: number) => Math.abs(Math.sin(i * k + 1.3))
    let up = i < 12 ? 20 + wave(0.9) * 40 : i < 27 ? 170 + wave(0.7) * 190 : i < 40 ? wave(1.7) * 12 : i === 55 ? 240 : 30 + wave(1.1) * 70
    let down = i < 40 ? wave(2.1) * 8 : 15 + wave(0.8) * 55
    if (i === 55) down = 30
    if (v === 'idle') { if (i > 53) { up = 0; down = 0 } else if (i > 27 && i < 48) up = i % 7 === 0 ? 20 : 0 }
    if (v === 'limited' && i >= 42) up = 150
    if (v === 'metered' && i >= 57) up = 0
    return { up: Math.round(up), down: Math.round(down) }
  })
}

export interface BwDevice {
  id: string; name: string; platform: string
  status: 'Syncing' | 'Up to date' | 'Paused' | "Couldn't sync"
  files?: number; doneBytes?: number; totalBytes?: number; speedBps?: number | null; reported: string
}
export const BW_DEVICES: BwDevice[] = [
  { id: 'pc', name: 'This PC', platform: 'Windows', status: 'Syncing', files: 12, doneBytes: 120 * MB, totalBytes: 168 * MB, speedBps: 3.2 * MB, reported: 'just now' },
  { id: 'iphone', name: "Anna's iPhone", platform: 'iPhone', status: 'Syncing', files: 2, doneBytes: 18 * MB, totalBytes: 40 * MB, speedBps: 410 * 1024, reported: '12 s ago' },
  { id: 'mac', name: "Anna's MacBook", platform: 'macOS', status: 'Up to date', reported: '3 min ago' },
  { id: 'ipad', name: "Anna's iPad", platform: 'iPadOS', status: 'Paused', reported: '1 hour ago' },
]

export interface Notice { id: string; day: string; icon: 'users' | 'share' | 'mail' | 'shield'; title: string; line: string; time: string; fresh?: boolean; href: string }
export const NOTICES: Notice[] = [
  { id: 'n1', day: 'Today', icon: 'shield', title: "New sign-in on Anna's iPad", line: 'Amsterdam, Netherlands · Beebeeb for iPadOS', time: '08:12', fresh: true, href: '?section=settings&page=devices' },
  { id: 'n2', day: 'Today', icon: 'share', title: 'Lotte de Wit shared Harbourlight shoot with you', line: 'Open it in Shared with me', time: '07:40', fresh: true, href: '?section=shared&sub=with-me' },
  { id: 'n3', day: 'Yesterday', icon: 'mail', title: 'Priya Shah asked for a file in Invoices 2026', line: 'Open the request to answer it', time: '16:05', href: '?section=shared&sub=requests' },
  { id: 'n4', day: '3 Nov', icon: 'shield', title: 'Security logging is on', line: 'Sign-ins and security changes are recorded', time: '10:31', href: '?section=settings&page=signin-activity' },
]

export type Kind = 'Media' | 'Documents' | 'Other'
export const INSIGHTS = {
  usedBytes: 240 * GB, quotaBytes: 1024 * GB, plan: 'Drive', files: 4424,
  byType: [
    { kind: 'Media' as Kind, bytes: 221.6 * GB, files: 3140 },
    { kind: 'Documents' as Kind, bytes: 13.9 * GB, files: 1032 },
    { kind: 'Other' as Kind, bytes: 4.5 * GB, files: 252 },
  ],
  largest: [
    { path: 'Backups/Anna-PC/Videos/2025/Harbourlight shoot/Render final.mov', kind: 'Video', bytes: 4.1 * GB },
    { path: 'Documents/Archive/Wedding 2025.zip', kind: 'Archive', bytes: 3.8 * GB },
    { path: 'Backups/Anna-PC/Projects/footage/Lisbon day 2.mp4', kind: 'Video', bytes: 3.2 * GB },
    { path: 'Photos/2024/Iceland/IMG_0001-IMG_0412.zip', kind: 'Archive', bytes: 2.6 * GB },
    { path: 'Backups/Anna-PC/Projects/Archive.tar', kind: 'Archive', bytes: 2.2 * GB },
    { path: 'Lisbon trip/Day 3/Drone 4K.mov', kind: 'Video', bytes: 1.9 * GB },
    { path: 'Documents/Invoices 2026/Scans 2026.pdf', kind: 'PDF', bytes: 1.4 * GB },
    { path: 'Backups/Anna-PC/Music/Sessions/Set 2023.wav', kind: 'Audio', bytes: 1.1 * GB },
    { path: 'Shoot selects/RAW/DSC_8821.NEF', kind: 'Photo', bytes: 880 * MB },
    { path: 'Documents/Notes/Interview recordings.m4a', kind: 'Audio', bytes: 640 * MB },
  ],
  largestTotal: 50,
}

/** INSIGHTS as `account_storage_breakdown` answers it, with the storage summary's numbers beside it: what the dev
 *  server's host reads back, so the live pane and its frames draw the board's page. The file ids are board files
 *  (./files.ts), so "Show in Files" lands on a real row in a browser; the paths stay the board's. */
const KIND_CATEGORY: Record<Kind, 'media' | 'documents' | 'other'> = { Media: 'media', Documents: 'documents', Other: 'other' }
const LARGEST_IDS = ['i-contract', 'i-agreement', 'p-tram', 'i-harbour', 'i-moodboard', 'p-alfama', 'd-initlabs', 'i-budget', 'p-belem', 'i-notes']
export const INSIGHTS_BREAKDOWN = {
  by_category: INSIGHTS.byType.map((t) => ({ category: KIND_CATEGORY[t.kind], bytes: t.bytes, file_count: t.files })),
  largest_files: INSIGHTS.largest.map((f, i) => ({ file_id: LARGEST_IDS[i], path: f.path, size_bytes: f.bytes, category: /PDF/.test(f.kind) ? 'documents' : /Archive/.test(f.kind) ? 'other' : 'media' })),
  total_files: INSIGHTS.files,
  total_bytes: INSIGHTS.usedBytes,
}

export interface KeepNode { id: string; name: string; gb: number; children?: KeepNode[] }
export const KEEP_TREE: KeepNode[] = [
  { id: 'backups', name: 'Backups', gb: 14.6 },
  { id: 'documents', name: 'Documents', gb: 1.4 },
  { id: 'invoices', name: 'Invoices 2026', gb: 0.04 },
  { id: 'lisbon', name: 'Lisbon trip', gb: 9.4, children: [
    { id: 'day1', name: 'Day 1', gb: 1.3 }, { id: 'day2', name: 'Day 2', gb: 3.9 }, { id: 'day3', name: 'Day 3', gb: 4.2 },
  ] },
  { id: 'selects', name: 'Shoot selects', gb: 3.8 },
]
export const KEEP_BEFORE = ['documents', 'invoices']
export const KEEP_AFTER = ['invoices', 'day1']

export interface Place { id: string; label: string; sub?: string; live: boolean }
export const PLACES: Place[] = [
  { id: 'default', label: 'Default', sub: 'Europe: Beebeeb places each new file in the EU', live: true },
  { id: 'de', label: 'Germany', live: true },
  { id: 'fi', label: 'Finland', live: true },
  { id: 'nl', label: 'Netherlands', sub: 'Coming soon', live: false },
]

export const RELEASE_NOTES = ['Faster first sync after you sign in on a new PC.', 'Backups now show which folder is waiting.', 'Fixes a case where File Explorer kept showing a file as syncing.']
export const CHANNEL_NEWEST: Record<'stable' | 'beta' | 'alpha', string> = { stable: '0.9.1', beta: '0.9.2', alpha: '0.9.3' }

// The speed test's board states (DW-0786a–d), moved here from the pane by P1c: board data, never the live app's.
export const SPEED_TEST_RESULT: SpeedResult = { latencyAvg: 24, latencyMin: 18, latencyMax: 41, up: 35.2, down: 94.0 }
export const TEST_STATES: Record<string, TestState> = {
  running: { phase: 'running', step: 'upload', done: 2, latency: 24, latencyRange: [18, 41] },
  downloads: { phase: 'running', step: 'download', done: 1, latency: 24, latencyRange: [18, 41], up: 35.2 },
  done: { phase: 'done', result: SPEED_TEST_RESULT },
  error: { phase: 'error', message: failureSentence('ping') },
}
