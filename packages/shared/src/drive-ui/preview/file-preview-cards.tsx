// ─── File preview state cards ───────────────────────
// The cards a preview shows instead of content: honest whole-file progress
// (LD-0001 PR-C), the over-cap and download prompts, the streaming-only
// error/changed states (LD-0001 PR-B, contract §4 rows 5-9), and the generic
// error card. Held here, not in file-preview.tsx, so that file keeps only the
// load orchestration and the renderer choice.

import { BBButton, Icon } from '../../design'
import { formatSize } from '../lib/format-size'
import { progressCopy } from '../lib/transfer-progress-copy'
import type { TransferProgress } from '../lib/transfer-progress-types'
import { editabilityNotice, type EditabilityResult } from '../lib/text-editability'

/** The one "there is no preview for this type" line, shared by both paths. */
export const UNSUPPORTED_TYPE_COPY = 'Preview not available for this file type'

/** Bytes saved so far by an over-cap download (`total` null before the size is known). */
export interface SavedProgress {
  written: number
  total: number | null
}

/** The whole-file loading state: what is arriving, and which part is opening. */
export function PreviewProgress({ transfer }: { transfer: TransferProgress | null }) {
  // Bar geometry: received is ciphertext progress; decrypted never exceeds it.
  const receivedPct =
    transfer && transfer.totalBytes && transfer.totalBytes > 0
      ? Math.min(100, (transfer.receivedBytes / transfer.totalBytes) * 100)
      : 0
  const decryptedPct =
    transfer && transfer.totalChunks > 0
      ? Math.min(receivedPct, (transfer.decryptedChunks / transfer.totalChunks) * 100)
      : 0
  return (
    <div className="flex flex-col items-center gap-3" data-testid="preview-transfer-progress">
      <div className="h-8 w-8 animate-spin rounded-full border-2 border-line border-t-amber" />
      <span className="text-sm text-ink-3">{transfer ? progressCopy(transfer) : 'Downloading…'}</span>
      {transfer && (
        <div className="w-64 space-y-1" aria-hidden="true">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-line">
            <div
              className="h-full bg-ink-3 transition-[width] duration-200"
              style={{ width: `${receivedPct}%` }}
            />
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-line">
            <div
              className="h-full bg-amber transition-[width] duration-200"
              style={{ width: `${decryptedPct}%` }}
            />
          </div>
        </div>
      )}
    </div>
  )
}

/** Over the 2 GiB whole-file media cap (contract §4 row 5): download only. */
export function OverCapCard({
  copy,
  downloading,
  savedProgress,
  onDownload,
}: {
  copy: string
  downloading: boolean
  savedProgress: SavedProgress | null
  onDownload: () => void
}) {
  return (
    <div className="flex flex-col items-center gap-4 text-center" data-testid="preview-too-large">
      <Icon name="file" size={40} className="text-ink-3" />
      <p className="max-w-[320px] text-sm text-ink-2">{copy}</p>
      <BBButton variant="amber" size="md" onClick={onDownload} disabled={downloading}>
        <Icon name="download" size={14} className="mr-2" />
        {downloading
          ? savedProgress && savedProgress.total
            ? `Downloading ${formatSize(savedProgress.written)} of ${formatSize(savedProgress.total)}`
            : 'Downloading…'
          : 'Download'}
      </BBButton>
    </div>
  )
}

/** "This file must be downloaded" — the streaming download card and the unsupported-type card. */
export function DownloadPromptCard({ copy, onDownload }: { copy: string; onDownload: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 text-center">
      <Icon name="file" size={40} className="text-ink-3" />
      <p className="max-w-[360px] text-sm text-ink-2">{copy}</p>
      <BBButton variant="amber" size="md" onClick={onDownload}>
        <Icon name="download" size={14} className="mr-2" />
        Download
      </BBButton>
    </div>
  )
}

/**
 * A streaming session that ended in error, or in `changed` (the file was
 * replaced). Only a retryable error offers "Try again": a replaced file has
 * nothing left to retry.
 */
export function StreamErrorCard({
  copy,
  retryable,
  onRetry,
}: {
  copy: string
  retryable: boolean
  onRetry: () => void
}) {
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <Icon name="shield" size={32} className="text-red" />
      <p className="max-w-[380px] text-sm font-medium text-ink">{copy}</p>
      {retryable && (
        <BBButton variant="default" size="sm" onClick={onRetry}>
          Try again
        </BBButton>
      )}
    </div>
  )
}

/**
 * A load that failed. A share-sourced file also offers to remove the item
 * (and explains how to get a copy), so the row stays actionable.
 */
export function PreviewErrorCard({
  message,
  shareSourced,
  onClose,
  onDelete,
}: {
  message: string
  shareSourced: boolean
  onClose: () => void
  onDelete?: () => void
}) {
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <Icon name={shareSourced ? 'lock' : 'shield'} size={32} className="text-red" />
      <p className="text-sm font-medium text-ink">{message}</p>
      {shareSourced && onDelete && (
        <p className="max-w-[320px] text-xs text-ink-3">
          You can remove the item from this folder, or ask the person who added it for a copy.
        </p>
      )}
      <div className="flex items-center gap-2">
        <BBButton variant="default" size="sm" onClick={onClose}>
          Close
        </BBButton>
        {shareSourced && onDelete && (
          <BBButton variant="danger" size="sm" onClick={onDelete}>
            Delete
          </BBButton>
        )}
      </div>
    </div>
  )
}

/**
 * Read-only editor notice (design screen 06): a markdown/code/text file that
 * failed the byte-level editability gate — too large, binary, or not UTF-8.
 */
export function ReadOnlyEditorNotice({
  editability,
  onDownload,
}: {
  editability: EditabilityResult
  onDownload: () => void
}) {
  return (
    <div
      className="flex w-full max-w-[520px] flex-col items-center gap-3 rounded-md border border-line bg-paper p-6 text-center shadow-1"
      data-testid="editor-readonly-notice"
    >
      <Icon name={editability.reason === 'binary' ? 'file' : 'shield'} size={26} className="text-ink-3" />
      <p className="text-sm font-semibold text-ink">{editabilityNotice(editability.reason).title}</p>
      <p className="text-[12.5px] leading-relaxed text-ink-3">{editabilityNotice(editability.reason).body}</p>
      <BBButton variant="amber" size="sm" onClick={onDownload}>
        <Icon name="download" size={13} className="mr-1.5" />
        Download
      </BBButton>
    </div>
  )
}
