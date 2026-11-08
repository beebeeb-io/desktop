/**
 * "You have unsaved changes" guard dialog (task 1563).
 *
 * Shown whenever the user tries to leave an unsaved edit — closing the
 * whole file preview, or the editor's own "Done" button — while the
 * in-memory draft differs from the last saved text. Nothing autosaves and
 * nothing is discarded silently.
 */

import { Icon, BBButton } from '../../design'

interface UnsavedChangesDialogProps {
  onDiscard: () => void
  onCancel: () => void
}

export function UnsavedChangesDialog({ onDiscard, onCancel }: UnsavedChangesDialogProps) {
  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="unsaved-dialog-title"
        data-testid="unsaved-changes-dialog"
        className="w-full max-w-[380px] rounded-lg border border-line bg-paper p-4 shadow-3"
      >
        <div className="flex items-start gap-3">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-amber-bg text-amber-deep">
            <Icon name="edit" size={14} />
          </span>
          <div>
            <h4 id="unsaved-dialog-title" className="text-[13.5px] font-semibold text-ink">
              Discard unsaved changes?
            </h4>
            <p className="mt-1 text-[12px] leading-relaxed text-ink-3">
              Your edits haven&rsquo;t been saved. They only live in this browser tab&rsquo;s
              memory — closing now loses them for good.
            </p>
          </div>
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <BBButton size="sm" variant="ghost" onClick={onCancel} data-testid="unsaved-keep-editing">
            Keep editing
          </BBButton>
          <BBButton size="sm" variant="danger" onClick={onDiscard} data-testid="unsaved-discard">
            Discard changes
          </BBButton>
        </div>
      </div>
    </div>
  )
}
