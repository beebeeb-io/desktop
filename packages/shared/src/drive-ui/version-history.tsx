// The version-history panel, view only. The host loads the versions, runs download, restore and delete,
// and passes the confirmation UI; this draws the timeline.
import { useEffect, type ReactNode } from 'react'
import { BBButton, Icon } from '../design'
import { formatBytes } from './lib/format'
import { useFocusTrap } from './hooks/use-focus-trap'

export interface VersionViewItem {
  id: string
  versionNumber: number
  createdAt: string
  /** Decrypted content size shown beside the version. */
  contentBytes: number
  canDelete: boolean
}

export interface VersionHistoryViewProps {
  open: boolean
  onClose: () => void
  fileName: string
  currentVersion: number
  versions: VersionViewItem[]
  loading: boolean
  selectedIdx: number
  onSelect: (index: number) => void
  /** An upgrade nudge for plans that keep only the current version. */
  upsell?: ReactNode
  onDownload: (versionId: string) => void
  onRestore: (versionId: string) => void
  pendingDeleteId: string | null
  onRequestDelete: (versionId: string) => void
  /** Drawn under the selected version while its delete is pending. */
  deleteConfirm?: ReactNode
}

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  return `${days}d ago`
}

export function VersionHistoryView({
  open,
  onClose,
  fileName,
  currentVersion,
  versions,
  loading,
  selectedIdx,
  onSelect,
  upsell,
  onDownload,
  onRestore,
  pendingDeleteId,
  onRequestDelete,
  deleteConfirm,
}: VersionHistoryViewProps) {
  const focusTrapRef = useFocusTrap<HTMLDivElement>(open)

  useEffect(() => {
    if (!open) return
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [open, onClose])

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex justify-end" onClick={onClose}>
      <div className="absolute inset-0 bg-ink/20" />

      <div
        ref={focusTrapRef}
        role="dialog"
        aria-modal="true"
        aria-label="Version history"
        onClick={(e) => e.stopPropagation()}
        className="relative w-full max-w-[540px] h-full bg-paper border-l border-line-2 shadow-3 overflow-hidden flex flex-col"
      >
        <div className="px-xl py-lg border-b border-line flex items-center gap-2.5">
          <Icon name="clock" size={14} className="text-amber-deep" />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-semibold text-ink">Version history</div>
            <div className="text-[11px] text-ink-3 mt-0.5 truncate">
              {fileName} — current v{currentVersion}
            </div>
          </div>
          <button
            onClick={onClose}
            aria-label="Close"
            className="w-7 h-7 rounded-md bg-paper-2 flex items-center justify-center text-ink-3 hover:text-ink transition-colors"
          >
            <Icon name="x" size={13} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto">
          {upsell && <div className="px-xl pt-md">{upsell}</div>}
          {loading ? (
            <div className="px-xl py-12 text-center text-sm text-ink-3">Loading versions...</div>
          ) : versions.length === 0 ? (
            <div className="px-xl py-12 text-center">
              <div className="text-sm text-ink-3">No previous versions</div>
              <div className="text-[11px] text-ink-4 mt-1">
                Versions are created when you re-upload a file with the same name
              </div>
            </div>
          ) : (
            <div className="py-1 relative" role="listbox" aria-label="Version history" aria-orientation="vertical">
              <div
                className="absolute bg-line-2"
                style={{ left: 28, top: 14, bottom: 14, width: 1 }}
              />

              {versions.map((v, i) => {
                const isSelected = i === selectedIdx
                return (
                  <div
                    key={v.id}
                    role="option"
                    aria-selected={isSelected}
                    aria-label={`Version ${v.versionNumber}, ${timeAgo(v.createdAt)}, ${formatBytes(v.contentBytes)}`}
                    className="w-full text-left relative cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-deep focus-visible:ring-inset"
                    style={{
                      padding: '12px 16px 12px 44px',
                      background: isSelected ? 'var(--color-paper-2)' : 'transparent',
                      borderLeft: isSelected
                        ? '3px solid var(--color-amber-deep)'
                        : '3px solid transparent',
                    }}
                    tabIndex={0}
                    onClick={() => onSelect(i)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        onSelect(i)
                      }
                    }}
                  >
                    <div
                      className="absolute rounded-full border-2"
                      style={{
                        left: 23,
                        top: 16,
                        width: 11,
                        height: 11,
                        background: isSelected ? 'var(--color-amber)' : 'var(--color-paper)',
                        borderColor: isSelected ? 'var(--color-amber-deep)' : 'var(--color-line-2)',
                      }}
                    />

                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[12px] font-semibold text-ink">
                        v{v.versionNumber}
                      </span>
                      <span className="text-[11px] text-ink-3">{timeAgo(v.createdAt)}</span>
                      <span className="text-[11px] font-mono text-ink-4">{formatBytes(v.contentBytes)}</span>
                    </div>

                    <div className="text-[11px] text-ink-4 mt-0.5 font-mono">
                      {new Date(v.createdAt).toLocaleString()}
                    </div>

                    {isSelected && (
                      <div className="flex gap-1.5 mt-2">
                        <BBButton
                          size="sm"
                          variant="ghost"
                          onClick={(e) => {
                            e.stopPropagation()
                            onDownload(v.id)
                          }}
                        >
                          Download
                        </BBButton>
                        <BBButton
                          size="sm"
                          variant="amber"
                          onClick={(e) => {
                            e.stopPropagation()
                            onRestore(v.id)
                          }}
                        >
                          Restore
                        </BBButton>
                        {v.canDelete && pendingDeleteId !== v.id && (
                          <BBButton
                            size="sm"
                            variant="danger"
                            onClick={(e) => {
                              e.stopPropagation()
                              onRequestDelete(v.id)
                            }}
                          >
                            Delete
                          </BBButton>
                        )}
                      </div>
                    )}

                    {isSelected && v.canDelete && pendingDeleteId === v.id && deleteConfirm}
                  </div>
                )
              })}
            </div>
          )}
        </div>

        <div className="px-xl py-sm border-t border-line bg-paper-2">
          <div className="flex items-center gap-1.5 text-[11px] text-ink-3">
            <Icon name="lock" size={10} className="text-amber" />
            Encrypted — versions stored with your file key
          </div>
        </div>
      </div>
    </div>
  )
}
