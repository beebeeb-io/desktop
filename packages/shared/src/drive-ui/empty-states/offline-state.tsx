// The empty state shown while the app is offline.
import { Icon } from '../../design'
import { BBButton } from '../../design'

interface QueuedChange {
  name: string
  status?: string
}

interface OfflineStateProps {
  queuedCount?: number
  queuedSize?: string
  changes?: QueuedChange[]
  onRetry?: () => void
  onSettings?: () => void
}

export function OfflineState({
  queuedCount = 0,
  queuedSize = '0 MB',
  changes = [],
  onRetry,
  onSettings,
}: OfflineStateProps) {
  return (
    <div
      className="w-[340px] bg-paper border border-line-2 rounded-xl overflow-hidden"
      style={{ boxShadow: 'var(--shadow-2)' }}
    >
      {/* Header */}
      <div className="px-3.5 py-3 border-b border-line flex items-center gap-2.5">
        <div className="w-[26px] h-[26px] rounded-full bg-paper-2 border border-line flex items-center justify-center text-ink-3 shrink-0">
          <Icon name="cloud" size={13} />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-[13px] font-semibold">
            Offline — working locally
          </div>
          <div className="font-mono text-[10.5px] text-ink-3">
            {queuedCount} change{queuedCount !== 1 ? 's' : ''} queued
            &middot; {queuedSize}
          </div>
        </div>
        {onSettings && (
          <BBButton size="sm" variant="ghost" onClick={onSettings}>
            <Icon name="settings" size={13} />
          </BBButton>
        )}
      </div>

      {/* Explanation */}
      <div className="px-3.5 pt-3.5 pb-2.5 text-xs text-ink-2 leading-relaxed">
        Your edits are encrypted and saved on disk. They&apos;ll sync to
        Beebeeb automatically when you reconnect.
      </div>

      {/* Queued changes list */}
      {changes.length > 0 && (
        <div className="px-3.5 pb-3">
          {changes.map((c, i) => (
            <div
              key={i}
              className="flex items-center gap-2 py-[5px] text-xs"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-amber shrink-0" />
              <span className="flex-1 truncate">{c.name}</span>
              <span className="font-mono text-[10px] text-ink-4 shrink-0">
                {c.status ?? 'queued'}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Footer */}
      <div className="px-3.5 py-2.5 border-t border-line bg-paper-2 flex items-center text-[11px] text-ink-3">
        <Icon name="lock" size={11} className="text-amber-deep" />
        <span className="ml-1.5">Local changes encrypted</span>
        {onRetry && (
          <span className="ml-auto">
            <BBButton size="sm" variant="ghost" onClick={onRetry}>
              Retry
            </BBButton>
          </span>
        )}
      </div>
    </div>
  )
}
