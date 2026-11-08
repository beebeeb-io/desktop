/**
 * OfficeConflictDialog (task 1567) — the binary-file sibling of
 * `../editor/conflict-dialog.tsx`. Same trigger (the server's version moved
 * on since this session opened the file) and same "never overwrite silently"
 * guarantee, but a byte diff of a .docx/.xlsx/.pptx means nothing to look
 * at, so per the task brief "Show differences" is replaced by "Discard".
 */

import { Icon, BBButton } from '../../design'
import type { OfficeConflictAction } from '../lib/office/office-conflict'

export interface OfficeConflictDialogProps {
  latestVersionNumber: number
  openedVersionNumber: number
  saving?: boolean
  onAction: (action: OfficeConflictAction) => void
  onCancel: () => void
}

export function OfficeConflictDialog({ latestVersionNumber, openedVersionNumber, saving, onAction, onCancel }: OfficeConflictDialogProps) {
  return (
    <div className="absolute inset-0 z-[6] flex items-center justify-center bg-black/30 dark:bg-black/55" data-testid="office-conflict-dialog">
      <div className="w-[440px] max-w-[90vw] rounded-xl border border-line bg-paper-2 p-5 shadow-3">
        <div className="mb-3 flex items-center gap-2.5">
          <Icon name="flag" size={16} className="text-amber-deep" />
          <h2 className="text-[15px] font-semibold text-ink">Someone else saved first</h2>
        </div>
        <p className="mb-5 text-[13px] leading-relaxed text-ink-2">
          This file was saved as version {latestVersionNumber} while you were editing (you opened version{' '}
          {openedVersionNumber}). Office documents can&apos;t be diffed line by line — choose how to keep your edit.
        </p>
        <div className="flex flex-col gap-2">
          <BBButton variant="amber" size="md" disabled={saving} onClick={() => onAction('save-as-new-version')} data-testid="office-conflict-save-new">
            Save as a new version anyway
          </BBButton>
          <BBButton variant="default" size="md" disabled={saving} onClick={() => onAction('keep-both')} data-testid="office-conflict-keep-both">
            Keep both — save mine as a separate file
          </BBButton>
          <button
            type="button"
            disabled={saving}
            onClick={() => onAction('discard')}
            data-testid="office-conflict-discard"
            className="rounded-md px-3 py-2 text-[13px] text-red hover:bg-red-bg disabled:opacity-40"
          >
            Discard my changes and reopen version {latestVersionNumber}
          </button>
        </div>
        <button type="button" onClick={onCancel} className="mt-3 w-full text-center text-[12px] text-ink-3 underline hover:text-ink-2">
          Cancel
        </button>
      </div>
    </div>
  )
}
