// Horizontal version timeline shown under the preview chrome when a file
// has more than one version. Clicking a dot selects that version and the
// FilePreview crossfades the content. The "current" version is the
// rightmost dot (newest); historical versions extend to the left.

import { Icon } from '../../design'

export interface ScrubberVersion {
  id: string
  version_number: number
  created_at: string
  size_bytes: number
}

interface VersionScrubberProps {
  versions: ScrubberVersion[]
  currentVersionNumber: number
  /** id of the version currently being viewed (null = current). */
  selectedVersionId: string | null
  onSelect: (versionId: string | null) => void
  /** Fires when user clicks "Restore this version" while viewing an older version. */
  onRestore?: () => void
  loading?: boolean
}

function shortDate(dateStr: string): string {
  const d = new Date(dateStr)
  const now = new Date()
  const diffMs = now.getTime() - d.getTime()
  const diffMin = Math.floor(diffMs / 60_000)
  if (diffMin < 1) return 'now'
  if (diffMin < 60) return `${diffMin}m`
  const diffHours = Math.floor(diffMin / 60)
  if (diffHours < 24) return `${diffHours}h`
  const diffDays = Math.floor(diffHours / 24)
  if (diffDays < 7) return `${diffDays}d`
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

export function VersionScrubber({
  versions,
  currentVersionNumber,
  selectedVersionId,
  onSelect,
  onRestore,
  loading = false,
}: VersionScrubberProps) {
  if (versions.length <= 1) return null

  // Server returns versions newest-first by convention; sort ascending so
  // the timeline reads left-to-right oldest → newest.
  const sorted = [...versions].sort((a, b) => a.version_number - b.version_number)

  // The "current" sentinel is null (rendered as the rightmost dot); a
  // selected historical version takes its id.
  const isViewingCurrent = selectedVersionId === null

  return (
    <div className="shrink-0 border-b border-line bg-paper-2 px-4 py-2.5">
      <div className="flex items-center gap-3">
        <span className="font-mono text-[10px] uppercase tracking-wider text-ink-3 shrink-0">
          Versions
        </span>

        {/* Track */}
        <div className="relative flex-1 min-w-0 h-7 flex items-center">
          {/* Hairline */}
          <div className="absolute left-0 right-0 h-px bg-line-2" />

          {/* Dots, evenly distributed */}
          <div className="relative flex w-full items-center justify-between">
            {sorted.map((v) => {
              const selected = !isViewingCurrent && selectedVersionId === v.id
              const isCurrent = v.version_number === currentVersionNumber
              // Current version: when isViewingCurrent, mark the rightmost dot active.
              const active = selected || (isCurrent && isViewingCurrent)
              return (
                <button
                  key={v.id}
                  type="button"
                  onClick={() => onSelect(isCurrent ? null : v.id)}
                  className="group relative flex flex-col items-center gap-0.5"
                  title={`v${v.version_number} · ${new Date(v.created_at).toLocaleString()}`}
                >
                  <span
                    className={`block rounded-full border transition-all ${
                      active
                        ? 'h-3 w-3 bg-amber border-amber-deep shadow-[0_0_0_3px_oklch(0.82_0.17_84/0.18)]'
                        : 'h-2.5 w-2.5 bg-paper border-line-2 group-hover:border-amber-deep'
                    }`}
                  />
                  <span
                    className={`font-mono text-[9.5px] tabular-nums ${
                      active ? 'text-amber-deep font-medium' : 'text-ink-4 group-hover:text-ink-3'
                    }`}
                  >
                    v{v.version_number}
                  </span>
                  <span className="font-mono text-[9px] text-ink-4 group-hover:text-ink-3">
                    {shortDate(v.created_at)}
                  </span>
                </button>
              )
            })}
          </div>
        </div>

        {/* Status / restore */}
        <div className="shrink-0 flex items-center gap-2">
          {loading && (
            <span className="font-mono text-[10.5px] text-ink-3 inline-flex items-center gap-1.5">
              <span className="inline-block h-2 w-2 rounded-full bg-amber-deep animate-pulse" />
              Loading
            </span>
          )}
          {!isViewingCurrent && !loading && (
            <button
              type="button"
              onClick={onRestore}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11.5px] bg-amber-bg text-amber-deep border border-amber-deep hover:bg-amber-bg/80 transition-colors cursor-pointer"
            >
              <Icon name="upload" size={11} className="rotate-180" />
              Restore this version
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
