/**
 * DuplicateFileDialog — shown before upload when one or more files share a
 * name with an existing file in the target folder.
 *
 * Single conflict:
 *   [Replace ★]  Creates a new version; previous version kept in history.
 *   [Keep both]  Uploads as "report (1).pdf" (or next available suffix).
 *   [Cancel]
 *
 * Multiple conflicts:
 *   "N files already exist"
 *   [Replace all ★]  [Keep both]  [Cancel]
 */

import { useEffect } from 'react'
import { useFocusTrap } from './hooks/use-focus-trap'
import { BBButton } from '../design'
import { Icon } from '../design'

export interface ConflictItem {
  /** The file the user is trying to upload. */
  newFile: File
  /** Existing file in the folder with the same name. */
  existingFileId: string
  existingName: string
}

interface DuplicateFileDialogProps {
  open: boolean
  conflicts: ConflictItem[]
  onReplace: () => void   // keep existing file_id → server auto-versions
  onKeepBoth: () => void  // new UUID, new name with "(N)" suffix
  onCancel: () => void
}

export function DuplicateFileDialog({
  open,
  conflicts,
  onReplace,
  onKeepBoth,
  onCancel,
}: DuplicateFileDialogProps) {
  const focusTrapRef = useFocusTrap<HTMLDivElement>(open)

  // Close on Escape
  useEffect(() => {
    if (!open) return
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [open, onCancel])

  if (!open) return null

  const isBulk = conflicts.length > 1
  const first = conflicts[0]

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center"
      onClick={onCancel}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-ink/20" />

      {/* Dialog */}
      <div
        ref={focusTrapRef}
        role="dialog"
        aria-modal="true"
        aria-label={isBulk ? 'Multiple files already exist' : `${first?.existingName ?? 'File'} already exists`}
        onClick={(e) => e.stopPropagation()}
        className="relative z-10 w-full max-w-[440px] mx-4 bg-paper border border-line-2 rounded-xl shadow-3 overflow-hidden"
      >
        {/* Header */}
        <div className="px-xl py-lg border-b border-line flex items-start gap-3">
          <div className="w-9 h-9 rounded-lg bg-amber-bg flex items-center justify-center shrink-0 mt-0.5">
            <Icon name="file" size={16} className="text-amber-deep" />
          </div>
          <div className="flex-1 min-w-0">
            {isBulk ? (
              <>
                <h2 className="text-[15px] font-semibold text-ink">
                  {conflicts.length} files already exist
                </h2>
                <p className="text-[12.5px] text-ink-3 mt-0.5">
                  {conflicts.length} of the files you're uploading share a name with existing files in this folder.
                </p>
              </>
            ) : (
              <>
                <h2 className="text-[15px] font-semibold text-ink break-all">
                  <span className="font-mono text-[14px]">{first?.existingName}</span>{' '}already exists
                </h2>
                <p className="text-[12.5px] text-ink-3 mt-0.5">
                  This file already exists in this folder.
                </p>
              </>
            )}
          </div>
          <button
            type="button"
            onClick={onCancel}
            aria-label="Cancel"
            className="text-ink-3 hover:text-ink transition-colors shrink-0"
          >
            <Icon name="x" size={14} />
          </button>
        </div>

        {/* Conflict list (for bulk) */}
        {isBulk && (
          <div className="max-h-[140px] overflow-y-auto border-b border-line">
            {conflicts.map((c, i) => (
              <div
                key={i}
                className="flex items-center gap-2 px-xl py-2 text-[12.5px] text-ink-2 border-b border-line last:border-b-0"
              >
                <Icon name="file" size={12} className="text-ink-4 shrink-0" />
                <span className="font-mono truncate flex-1">{c.existingName}</span>
              </div>
            ))}
          </div>
        )}

        {/* Actions */}
        <div className="px-xl py-lg space-y-2.5">
          {/* Replace (primary — default action) */}
          <button
            type="button"
            onClick={onReplace}
            className="w-full flex items-start gap-3 px-4 py-3.5 rounded-lg border-2 border-amber bg-amber-bg hover:bg-amber/10 transition-colors text-left group cursor-pointer"
          >
            <div className="w-6 h-6 rounded-full bg-amber flex items-center justify-center shrink-0 mt-0.5">
              <Icon name="upload" size={12} className="text-ink" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-[13.5px] font-semibold text-ink">
                {isBulk ? 'Replace all' : 'Replace'}
              </div>
              <div className="text-[11.5px] text-ink-3 mt-0.5 leading-relaxed">
                Creates a new version. The previous {isBulk ? 'versions are' : 'version is'} saved in version history.
              </div>
            </div>
            <span className="text-[10px] font-semibold text-amber-deep bg-amber/20 border border-amber/30 rounded px-1.5 py-0.5 shrink-0 self-start mt-0.5">
              DEFAULT
            </span>
          </button>

          {/* Keep both */}
          <button
            type="button"
            onClick={onKeepBoth}
            className="w-full flex items-start gap-3 px-4 py-3.5 rounded-lg border border-line hover:border-line-2 hover:bg-paper-2 transition-colors text-left cursor-pointer"
          >
            <div className="w-6 h-6 rounded-full bg-paper-3 flex items-center justify-center shrink-0 mt-0.5">
              <Icon name="plus" size={12} className="text-ink-3" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-[13.5px] font-semibold text-ink">Keep both</div>
              <div className="text-[11.5px] text-ink-3 mt-0.5 leading-relaxed">
                {isBulk
                  ? 'Uploads with a "(1)" suffix — e.g. "report (1).pdf".'
                  : `Uploads as "${addSuffix(first?.existingName ?? '')}" — the existing file is unchanged.`}
              </div>
            </div>
          </button>

          {/* Cancel */}
          <BBButton
            variant="ghost"
            size="md"
            className="w-full justify-center"
            onClick={onCancel}
          >
            Cancel
          </BBButton>
        </div>
      </div>
    </div>
  )
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Preview what the "(1)" suffix would look like for a given filename. */
function addSuffix(name: string, n = 1): string {
  const dot = name.lastIndexOf('.')
  const base = dot > 0 ? name.slice(0, dot) : name
  const ext = dot > 0 ? name.slice(dot) : ''
  return `${base} (${n})${ext}`
}

/**
 * Return a unique name by appending (1), (2), … until there's no collision
 * with either existing folder files or other files already queued for upload.
 */
export function getUniqueName(
  name: string,
  existingNames: ReadonlySet<string>,  // lowercase names of files already in folder
  usedNames: ReadonlySet<string>,       // lowercase names taken by other uploads in this batch
): string {
  const dot = name.lastIndexOf('.')
  const base = dot > 0 ? name.slice(0, dot) : name
  const ext = dot > 0 ? name.slice(dot) : ''

  let n = 1
  let candidate = name
  while (existingNames.has(candidate.toLowerCase()) || usedNames.has(candidate.toLowerCase())) {
    candidate = `${base} (${n})${ext}`
    n++
  }
  return candidate
}
