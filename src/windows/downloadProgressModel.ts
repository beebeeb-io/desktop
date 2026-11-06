/**
 * Pure view functions for the downloading / streaming progress row (LD-0105 U1).
 * No side-effects, no imports from React or Tauri — safe to use in Bun tests.
 *
 * The IPC DTO shape is defined in `desktopApi.ts` (`ActiveTransferRow`).
 * The strings come from `downloadProgressCopy.ts`.
 */

import { TRAY_SUBLINE, ACTIVITY_DETAIL } from './downloadProgressCopy'

// ── Formatting helpers ────────────────────────────────────────────────────────

/** Format a speed in bytes/s as a human-readable string, e.g. "9.6 MB/s". */
export function formatSpeed(speedBps: number): string {
  if (speedBps <= 0) return '0 B/s'
  if (speedBps < 1_024) return `${speedBps} B/s`
  if (speedBps < 1_048_576) return `${(speedBps / 1_024).toFixed(1)} KB/s`
  if (speedBps < 1_073_741_824) return `${(speedBps / 1_048_576).toFixed(1)} MB/s`
  return `${(speedBps / 1_073_741_824).toFixed(1)} GB/s`
}

/** Format a 0–100 fraction as "34%" (no space before the sign). */
export function formatPercent(done: number, total: number): string {
  if (total <= 0) return '0%'
  return `${Math.min(100, Math.round((done / total) * 100))}%`
}

/** Format bytes as a compact size string, e.g. "9.6 MB". */
export function formatSizeText(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`
  if (bytes < 1_073_741_824) return `${(bytes / 1_048_576).toFixed(1)} MB`
  return `${(bytes / 1_073_741_824).toFixed(1)} GB`
}

// ── Transfer state classification ─────────────────────────────────────────────

/** The four UI states of a downloading / streaming transfer. */
export type TransferUiState = 'downloading' | 'streaming' | 'buffering' | 'failed'

export interface ActiveTransferRowLike {
  direction: 'up' | 'down' | 'stream'
  done: number
  total: number
  buffered: number
  speedBps: number
  failed: boolean
  failedAtSecs?: number | null
}

/** Classify a transfer into one of the four UI states. */
export function transferUiState(row: ActiveTransferRowLike): TransferUiState {
  if (row.failed) return 'failed'
  if (row.direction === 'stream') {
    // Streaming: buffered bytes as percent of total (if known).
    // If total is unknown or zero, show "Buffering" until we have a meaningful percent.
    const hasPercent = row.total > 0 && row.buffered > 0
    return hasPercent ? 'streaming' : 'buffering'
  }
  return 'downloading'
}

// ── Tray subline ──────────────────────────────────────────────────────────────

/**
 * The one-line sub-label shown beneath the filename in the tray's Recent list.
 * "Couldn't finish downloading" for failed; never includes "complete".
 */
export function traySubline(row: ActiveTransferRowLike): string {
  const state = transferUiState(row)
  if (state === 'failed') return TRAY_SUBLINE.failed
  const speed = formatSpeed(row.speedBps)
  if (state === 'buffering') return TRAY_SUBLINE.buffering(speed)
  if (state === 'streaming') {
    const pct = formatPercent(row.buffered, row.total)
    return TRAY_SUBLINE.streaming(pct, speed)
  }
  // downloading
  const pct = formatPercent(row.done, row.total)
  return TRAY_SUBLINE.downloading(pct, speed)
}

// ── Activity detail ───────────────────────────────────────────────────────────

/**
 * The longer sentence shown in Activity › Timeline and in the tray tooltip.
 * Failed state never says "complete".
 */
export function activityDetail(row: ActiveTransferRowLike): string {
  const state = transferUiState(row)
  if (state === 'failed') return ACTIVITY_DETAIL.failed
  const speed = formatSpeed(row.speedBps)
  if (state === 'buffering') return ACTIVITY_DETAIL.buffering(speed)
  if (state === 'streaming') {
    const pct = formatPercent(row.buffered, row.total)
    return ACTIVITY_DETAIL.streaming(pct, speed)
  }
  const pct = formatPercent(row.done, row.total)
  return ACTIVITY_DETAIL.downloading(pct, speed)
}

// ── Progress bar fraction ─────────────────────────────────────────────────────

/**
 * 0–1 fill fraction for the progress bar. For streaming: buffered/total.
 * For failed: `null` (no bar). For downloading: done/total.
 */
export function progressFraction(row: ActiveTransferRowLike): number | null {
  if (row.failed) return null
  if (row.direction === 'stream') {
    return row.total > 0 ? Math.min(1, row.buffered / row.total) : null
  }
  return row.total > 0 ? Math.min(1, row.done / row.total) : null
}

// ── "In progress" card visibility ────────────────────────────────────────────

/** How long (ms) a failed transfer's card stays visible after failure. */
const FAILED_ROW_TTL_MS = 15 * 60 * 1_000

/**
 * Whether the "In progress" card should be shown. `nowMs` defaults to `Date.now()`.
 * Visible when any transfer is active, or when a failed transfer happened within
 * the last 15 minutes.
 */
export function shouldShowInProgressCard(
  rows: ActiveTransferRowLike[],
  nowMs = Date.now(),
): boolean {
  return rows.some((r) => {
    if (!r.failed) return true
    if (r.failedAtSecs == null) return true
    return nowMs - r.failedAtSecs * 1_000 < FAILED_ROW_TTL_MS
  })
}
