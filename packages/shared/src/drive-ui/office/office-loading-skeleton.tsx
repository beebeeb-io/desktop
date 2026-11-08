/**
 * Office editor loading skeleton (task 1567 — web delivery groundwork).
 *
 * The "instant first page": while the office WASM bundle streams in (tens of
 * seconds even on a fast link — 257 MB, see repos/office/docs/PHASE2-RESULTS.md),
 * this shows the document's EXISTING preview thumbnail immediately (from the
 * same decrypted-thumbnail pipeline `preview.tsx` already uses,
 * `fetchAndDecryptThumbnail` / `fetchAndDecryptLargeThumbnail` in
 * `src/lib/thumbnail.ts`) instead of a blank screen or a generic spinner.
 *
 * Honest copy only: this never claims "loading" or a false sense of near-
 * instant readiness. "Preparing the editor on this device…" says what is
 * actually happening (decryption + a large one-time-per-cache download run
 * locally) — see CLAUDE.md "Voice: Honest over reassuring".
 *
 * This component is presentational only — it does not know how to fetch a
 * thumbnail or load the office bundle. A caller passes the already-resolved
 * thumbnail URL (or null) and progress state; `loadOfficeBundle` (../../lib/office/loader)
 * is what a real integration would wire to `progress`/`onCancel`.
 */

import type { ReactNode } from 'react'
import { officeLoadPercent, officeLoadByteLabel } from '../lib/office/progress-format'

export type OfficeSkeletonStage = 'preparing' | 'ready' | 'error'

export interface OfficeLoadingSkeletonProps {
  /** Decrypted thumbnail object URL, or null if none exists yet (e.g. a brand-new blank document). */
  thumbnailUrl: string | null
  /** Decrypted filename, for the alt text / accessible label only. */
  filename: string
  stage: OfficeSkeletonStage
  /** Bytes streamed so far / total, once known. Omit while the manifest hasn't resolved yet. */
  progress?: { loadedBytes: number; totalBytes: number } | null
  /** Present only once a real cancel is wired up (e.g. to `createCancellableOfficeLoad().cancel`). */
  onCancel?: () => void
  /** Shown when stage === 'error'. */
  errorMessage?: string | null
  onRetry?: () => void
  /** The live editor canvas, mounted once stage === 'ready'. Rendered underneath so the
   *  crossfade never shows a blank frame between skeleton and canvas. */
  children?: ReactNode
}

export function OfficeLoadingSkeleton({
  thumbnailUrl,
  filename,
  stage,
  progress,
  onCancel,
  errorMessage,
  onRetry,
  children,
}: OfficeLoadingSkeletonProps) {
  const showOverlay = stage !== 'ready'
  const percent = progress ? officeLoadPercent(progress.loadedBytes, progress.totalBytes) : null
  const byteLabel = progress ? officeLoadByteLabel(progress.loadedBytes, progress.totalBytes) : null

  return (
    <div className="relative h-full w-full bg-paper-2" data-office-stage={stage}>
      {children}

      {showOverlay && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-5 bg-paper-2">
          {thumbnailUrl ? (
            <img
              src={thumbnailUrl}
              alt={`Preview of ${filename}`}
              className="max-h-[60%] max-w-[70%] rounded-md border border-line object-contain shadow-2"
              style={{ filter: stage === 'preparing' ? 'saturate(0.85) brightness(0.97)' : undefined }}
            />
          ) : (
            <div className="flex h-40 w-32 items-center justify-center rounded-md border border-line bg-paper shadow-1" />
          )}

          {stage === 'preparing' && (
            <div className="flex flex-col items-center gap-2.5">
              <div className="flex items-center gap-2.5">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-line border-t-amber" />
                <span className="text-[13px] text-ink-2">Preparing the editor on this device…</span>
              </div>
              {percent !== null && percent > 0 && (
                <div className="flex w-56 flex-col gap-1">
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-line">
                    <div
                      className="h-full rounded-full bg-amber transition-[width]"
                      style={{ width: `${percent}%` }}
                      role="progressbar"
                      aria-valuenow={percent}
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-label="Office editor download progress"
                    />
                  </div>
                  {byteLabel && (
                    <span className="text-center text-[11px] font-mono text-ink-3">{byteLabel}</span>
                  )}
                </div>
              )}
              {onCancel && (
                <button
                  onClick={onCancel}
                  className="text-[12px] text-ink-3 underline hover:text-ink-2"
                >
                  Cancel
                </button>
              )}
            </div>
          )}

          {stage === 'error' && (
            <div className="flex flex-col items-center gap-3 text-center">
              <span className="text-[13px] text-ink-2">
                {errorMessage ?? "We couldn't prepare the editor on this device."}
              </span>
              {onRetry && (
                <button
                  onClick={onRetry}
                  className="text-[12.5px] font-medium text-amber-deep underline hover:no-underline"
                >
                  Try again
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
