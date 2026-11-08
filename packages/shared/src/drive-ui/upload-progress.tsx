// The upload-progress list with per-item status and cancel.
import { Icon } from '../design'
import { formatBytes } from './lib/format'

export interface UploadItem {
  id: string
  name: string
  size: number
  progress: number
  stage: 'Queued' | 'Preparing' | 'Encrypting' | 'Uploading' | 'Done' | 'Error'
  /** Bytes uploaded so far — used for speed calculation */
  bytesUploaded?: number
  uploadedChunks?: number
  totalChunks?: number
  chunkSizeBytes?: number
  storageRegion?: string | null
  /** Timestamp when uploading stage started */
  startedAt?: number
  /** Whether this upload is paused */
  paused?: boolean
  /** Country where the ciphertext is stored (task 2050), when known — used in stage labels. Never a city. */
  storageCountry?: string
  /** Human-readable failure reason — surfaced when stage === 'Error' */
  errorMessage?: string
}

interface UploadProgressProps {
  items: UploadItem[]
  onClose: () => void
  onCancel?: (id: string) => void
  onPause?: (id: string) => void
  onResume?: (id: string) => void
  /** Retry a failed upload — only invoked for items in stage='Error' */
  onRetry?: (id: string) => void
}

function formatSpeed(bytesPerSecond: number): string {
  if (bytesPerSecond < 1024) return `${bytesPerSecond.toFixed(0)} B/s`
  if (bytesPerSecond < 1024 * 1024) return `${(bytesPerSecond / 1024).toFixed(1)} KB/s`
  return `${(bytesPerSecond / (1024 * 1024)).toFixed(1)} MB/s`
}

function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return '--'
  if (seconds < 60) return `${Math.ceil(seconds)}s`
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60)
    const s = Math.ceil(seconds % 60)
    return s > 0 ? `${m}m ${s}s` : `${m}m`
  }
  const h = Math.floor(seconds / 3600)
  const m = Math.ceil((seconds % 3600) / 60)
  return m > 0 ? `${h}h ${m}m` : `${h}h`
}

function computeSpeed(item: UploadItem): number {
  if (!item.startedAt || !item.bytesUploaded || item.bytesUploaded === 0) return 0
  const elapsed = (Date.now() - item.startedAt) / 1000
  if (elapsed <= 0) return 0
  return item.bytesUploaded / elapsed
}

function computeEta(item: UploadItem, speed: number): number {
  if (speed <= 0) return Infinity
  const remaining = item.size - (item.bytesUploaded ?? 0)
  return remaining / speed
}

type PipelineStage = 'encrypt' | 'upload' | 'confirm'

function pipelineStage(item: UploadItem): PipelineStage {
  if (item.stage === 'Done') return 'confirm'
  if (item.stage === 'Uploading') return 'upload'
  // Queued + Encrypting + Error all sit in the encrypt slot for the meter,
  // but stageLabel() reads item.stage directly so Error gets distinct copy.
  return 'encrypt'
}

function stageLabel(item: UploadItem): string {
  if (item.paused) return 'Paused'
  if (item.stage === 'Error') {
    return item.errorMessage ?? "Couldn't reach the server. Retry?"
  }
  if (item.stage === 'Queued') return 'Queued'
  if (item.stage === 'Preparing') return 'Preparing encrypted backup'
  switch (pipelineStage(item)) {
    case 'encrypt':
      return 'Encrypting on your device...'
    case 'upload':
      if (item.totalChunks) {
        return `Uploading ${item.uploadedChunks ?? 0} / ${item.totalChunks} chunks`
      }
      return 'Uploading ciphertext'
    case 'confirm':
      return `Stored in ${item.storageCountry ?? item.storageRegion ?? 'Europe'} · Key stayed on your device`
  }
}

export function UploadProgress({
  items,
  onClose,
  onCancel,
  onPause,
  onResume,
  onRetry,
}: UploadProgressProps) {
  if (items.length === 0) return null

  const totalBytes = items.reduce((s, i) => s + i.size, 0)
  const uploadedBytes = items.reduce((s, i) => s + (i.bytesUploaded ?? i.size * (i.progress / 100)), 0)
  const activeItems = items.filter((i) => i.stage === 'Uploading' && !i.paused)
  const totalSpeed = activeItems.reduce((s, i) => s + computeSpeed(i), 0)

  return (
    <div className="fixed bottom-4 right-4 z-40 w-[420px] bg-paper border border-line-2 rounded-xl shadow-3 overflow-hidden">
      {/* Header */}
      <div className="px-5 py-3.5 border-b border-line flex items-center">
        <span className="text-[13px] font-semibold">
          Uploading {items.length} file{items.length > 1 ? 's' : ''}
        </span>
        <span className="font-mono text-[11px] text-ink-3 ml-auto mr-3">
          {formatBytes(uploadedBytes)} / {formatBytes(totalBytes)}
        </span>
        <button
          onClick={onClose}
          aria-label="Close"
          className="p-0.5 text-ink-3 hover:text-ink transition-colors"
        >
          <Icon name="x" size={14} />
        </button>
      </div>

      {/* Aggregate speed */}
      {totalSpeed > 0 && (
        <div className="px-5 pt-2.5 flex items-center gap-3">
          <span className="font-mono text-[10.5px] text-ink-3">
            {formatSpeed(totalSpeed)}
          </span>
        </div>
      )}

      {/* File list */}
      <div className="px-5 py-3.5 flex flex-col gap-3.5 max-h-[340px] overflow-y-auto">
        {items.map((item) => {
          const speed = computeSpeed(item)
          const eta = computeEta(item, speed)
          const isActive = item.stage === 'Uploading' || item.stage === 'Encrypting'
          const isDone = item.stage === 'Done'
          const isError = item.stage === 'Error'
          const stage = pipelineStage(item)

          return (
            <div key={item.id}>
              <div className="flex items-center gap-2 mb-1">
                {/* File icon / done checkmark / error mark */}
                <div
                  className={`w-[18px] h-[18px] shrink-0 rounded-[5px] flex items-center justify-center ${isDone ? 'decrypt-pulse' : ''}`}
                  style={{
                    background: isDone
                      ? 'var(--color-amber-bg)'
                      : isError
                        ? 'color-mix(in oklch, var(--color-red) 12%, var(--color-paper))'
                        : 'var(--color-paper-2)',
                    border: isDone
                      ? '1px solid var(--color-amber)'
                      : isError
                        ? '1px solid var(--color-red)'
                        : '1px solid var(--color-line)',
                  }}
                >
                  {isDone ? (
                    <Icon name="check" size={11} className="text-amber-deep" />
                  ) : isError ? (
                    <Icon name="x" size={11} className="text-red" />
                  ) : (
                    <Icon name="file" size={10} className="text-ink-3" />
                  )}
                </div>

                {/* File name */}
                <span className="text-[12.5px] font-medium truncate">{item.name}</span>

                {/* Percentage / Failed badge */}
                {isError ? (
                  <span
                    className="font-mono text-[10.5px] font-medium ml-auto shrink-0 tabular-nums uppercase tracking-wider text-red"
                  >
                    Failed
                  </span>
                ) : (
                  <span
                    className="font-mono text-[10.5px] font-medium ml-auto shrink-0 tabular-nums"
                    style={{
                      color: isDone
                        ? 'var(--color-amber-deep)'
                        : item.paused
                          ? 'var(--color-ink-4)'
                          : item.stage === 'Queued'
                            ? 'var(--color-ink-4)'
                            : 'var(--color-amber-deep)',
                    }}
                  >
                    {item.progress}%
                  </span>
                )}

                {/* Retry button (only on Error) */}
                {isError && onRetry && (
                  <button
                    onClick={() => onRetry(item.id)}
                    aria-label="Retry upload"
                    title="Retry upload"
                    className="text-[10.5px] font-medium uppercase tracking-wider text-ink-3 hover:text-amber-deep transition-colors shrink-0"
                  >
                    Retry
                  </button>
                )}

                {/* Pause/Resume button */}
                {isActive && !isDone && !isError && onPause && onResume && (
                  <button
                    onClick={() => item.paused ? onResume(item.id) : onPause(item.id)}
                    aria-label={item.paused ? 'Resume upload' : 'Pause upload'}
                    className="p-0.5 text-ink-3 hover:text-ink transition-colors shrink-0"
                  >
                    <Icon name={item.paused ? 'play' : 'pause'} size={12} />
                  </button>
                )}

                {/* Cancel / Dismiss button */}
                {!isDone && onCancel && (
                  <button
                    onClick={() => onCancel(item.id)}
                    aria-label={isError ? 'Dismiss failed upload' : 'Cancel upload'}
                    className="p-0.5 text-ink-3 hover:text-red transition-colors shrink-0"
                  >
                    <Icon name="x" size={12} />
                  </button>
                )}
              </div>

              {/* Stage label (red on Error, amber-deep on encrypt/confirm) */}
              <div
                className="ml-[26px] mb-1.5 text-[11px] truncate"
                style={{
                  color: isError
                    ? 'var(--color-red)'
                    : stage === 'encrypt'
                      ? 'var(--color-amber-deep)'
                      : stage === 'confirm'
                        ? 'var(--color-amber-deep)'
                        : 'var(--color-ink-3)',
                }}
              >
                {stageLabel(item)}
              </div>

              {/* 3-stage pipeline bar */}
              <div className="ml-[26px] flex items-center gap-1.5">
                <StageBar
                  stage="encrypt"
                  current={stage}
                  // Encrypt fills as soon as we leave the encrypt slot
                  filled={stage !== 'encrypt' || (item.stage === 'Encrypting' && item.progress > 0)}
                  amber
                />
                <StageBar
                  stage="upload"
                  current={stage}
                  // Upload fills proportionally to upload progress
                  progress={
                    stage === 'upload'
                      ? Math.max(0, Math.min(100, item.progress))
                      : stage === 'confirm'
                        ? 100
                        : 0
                  }
                />
                <StageBar
                  stage="confirm"
                  current={stage}
                  filled={stage === 'confirm'}
                  amber
                  flash={stage === 'confirm'}
                />
              </div>

              {/* Speed + ETA row */}
              {item.stage === 'Uploading' && !isDone && !item.paused && speed > 0 && (
                <div className="flex items-center gap-2 mt-1.5 ml-[26px]">
                  <span className="font-mono text-[10px] text-ink-4">
                    {formatSpeed(speed)}
                  </span>
                  <span className="font-mono text-[10px] text-ink-4">
                    {formatEta(eta)} remaining
                  </span>
                </div>
              )}
            </div>
          )
        })}
      </div>

      {/* Info banner */}
      <div className="mx-5 mb-4 px-3 py-2.5 bg-amber-bg border border-line rounded-md flex items-center gap-2.5">
        <Icon name="key" size={14} className="text-amber-deep shrink-0" />
        <span className="text-xs" style={{ color: 'oklch(0.35 0.06 72)' }}>
          Keys stay on your device — never on our servers.
        </span>
      </div>
    </div>
  )
}

interface StageBarProps {
  stage: PipelineStage
  current: PipelineStage
  /** Either fully on (true) or use `progress` for partial fill */
  filled?: boolean
  /** Optional 0..100 partial fill */
  progress?: number
  /** Use amber rather than neutral fill */
  amber?: boolean
  /** Brief amber pulse animation when reaching this stage */
  flash?: boolean
}

const ORDER: Record<PipelineStage, number> = { encrypt: 0, upload: 1, confirm: 2 }

function StageBar({ stage, current, filled, progress, amber, flash }: StageBarProps) {
  const isPast = ORDER[stage] < ORDER[current]
  const fill =
    progress !== undefined
      ? progress
      : (filled || isPast)
        ? 100
        : 0

  const color = amber || isPast ? 'var(--color-amber)' : 'var(--color-ink-3)'

  return (
    <div
      className={`h-[3px] flex-1 rounded-full bg-paper-3 overflow-hidden ${flash ? 'decrypt-pulse' : ''}`}
    >
      <div
        className="h-full rounded-full transition-all duration-300"
        style={{
          width: `${fill}%`,
          background: color,
        }}
      />
    </div>
  )
}
