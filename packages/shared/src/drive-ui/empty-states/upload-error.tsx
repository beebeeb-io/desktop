// The failed-upload card and its retry/remove actions.
import { Icon } from '../../design'
import { BBButton } from '../../design'
import { BBChip } from '../../design'

export interface FailedUpload {
  name: string
  size: string
  progress: number
  stage: string
  tone: 'ok' | 'warn' | 'err'
}

interface UploadErrorProps {
  /** Where the upload was going (a country or "Europe"), when known. Never a city. */
  region?: string
  files: FailedUpload[]
  onResume: () => void
  onCancel: () => void
  onViewDetails?: () => void
}

export function UploadError({
  region,
  files,
  onResume,
  onCancel,
  onViewDetails,
}: UploadErrorProps) {
  const failedCount = files.filter((f) => f.tone === 'err').length
  const pausedCount = files.filter((f) => f.tone === 'warn').length

  return (
    <div className="w-full max-w-[520px] bg-paper border border-line-2 rounded-xl overflow-hidden" style={{ boxShadow: 'var(--shadow-2)' }}>
      {/* Header */}
      <div className="px-5 py-3.5 border-b border-line flex items-center">
        <h3 className="text-[15px] font-semibold">Upload</h3>
        <BBChip>
          <span className="flex items-center gap-1.5">
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{ background: 'var(--color-red)' }}
            />
            <span style={{ color: 'var(--color-red)' }}>
              {failedCount} failed
            </span>
            {pausedCount > 0 && (
              <span className="text-ink-3">&middot; {pausedCount} paused</span>
            )}
          </span>
        </BBChip>
      </div>

      <div className="px-5 py-3.5">
        {/* Warning banner */}
        <div className="flex items-start gap-2.5 p-3 mb-3.5 rounded-md bg-red-bg border border-red-border">
          <div className="w-[22px] h-[22px] rounded-full shrink-0 flex items-center justify-center bg-red/10 text-red">
            <Icon name="more" size={11} />
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-[13px] font-semibold text-red">
              {region ? `Connection to ${region} dropped` : 'Connection dropped'}
            </div>
            <div className="text-xs text-ink-2 mt-0.5 leading-relaxed">
              Chunks already encrypted stay safe. We can pick up where we
              left off — no re-encryption needed.
            </div>
          </div>
        </div>

        {/* File list */}
        <div className="flex flex-col gap-2.5">
          {files.map((f, i) => {
            const stageColor =
              f.tone === 'ok'
                ? 'var(--color-green)'
                : f.tone === 'warn'
                  ? 'var(--color-ink-3)'
                  : 'var(--color-red)'
            const barBg =
              f.tone === 'err'
                ? 'var(--color-red)'
                : f.tone === 'ok'
                  ? 'var(--color-ink)'
                  : 'var(--color-amber)'

            return (
              <div key={i}>
                <div className="flex items-center gap-2 mb-1.5">
                  <Icon name="file" size={12} className="text-ink-3" />
                  <span className="text-[12.5px] font-medium truncate">
                    {f.name}
                  </span>
                  <span className="font-mono text-[10.5px] text-ink-4">
                    {f.size}
                  </span>
                  <span
                    className="font-mono text-[10.5px] font-medium ml-auto"
                    style={{ color: stageColor }}
                  >
                    {f.stage}
                  </span>
                </div>
                <div className="h-[3px] w-full rounded-full bg-paper-3 overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all duration-300"
                    style={{ width: `${f.progress}%`, background: barBg }}
                  />
                </div>
              </div>
            )
          })}
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-4">
          <BBButton variant="amber" onClick={onResume} className="gap-1.5">
            <Icon name="arrow-up" size={12} /> Resume all
          </BBButton>
          {onViewDetails && (
            <BBButton size="sm" variant="ghost" onClick={onViewDetails}>
              View error details
            </BBButton>
          )}
          <BBButton
            size="sm"
            variant="ghost"
            className="ml-auto"
            onClick={onCancel}
          >
            Cancel
          </BBButton>
        </div>
      </div>
    </div>
  )
}
