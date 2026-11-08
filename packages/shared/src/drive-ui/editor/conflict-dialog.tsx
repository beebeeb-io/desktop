/**
 * Save-conflict dialog (task 1563, design/editor-1563.html screen 04).
 *
 * Shown when `getFile()` reports the server's version has moved past the
 * version this edit session opened from — i.e. another device/tab saved
 * first. Nothing is ever overwritten silently: the user picks one of three
 * actions (editor-conflict.ts holds the pure decision logic these buttons
 * drive) — Keep both / Save as a new version / Show differences.
 */

import { Fragment, useState } from 'react'
import { Icon, BBButton } from '../../design'
import {
  diffLines,
  toSideBySideRows,
  MAX_DIFF_LINES,
  type DiffLine,
  type SideCell,
} from '../lib/line-diff'
import type { ConflictAction } from '../lib/editor-conflict'

/** One side of a side-by-side diff row: a line-numbered gutter + the text,
 *  tinted green (add) / red (del), or a blank filler when the OTHER side has
 *  a line this side doesn't. */
function SideCellView({ cell, border }: { cell: SideCell; border: 'left' | 'right' }) {
  const bg =
    cell.type === 'add'
      ? 'color-mix(in oklab, var(--color-green) 12%, transparent)'
      : cell.type === 'del'
        ? 'color-mix(in oklab, var(--color-red) 10%, transparent)'
        : undefined
  return (
    <div
      className={`grid grid-cols-[2.5em_1fr] ${border === 'left' ? 'border-r border-line' : ''}`}
      style={{ backgroundColor: bg }}
    >
      <span className="select-none px-2 text-right text-ink-4">{cell.lineNo ?? ''}</span>
      <span className="whitespace-pre-wrap break-all px-2 text-ink-2">
        {cell.type === 'empty' ? '​' : cell.text || '​'}
      </span>
    </div>
  )
}

interface ConflictDialogProps {
  /** The version number that raced this edit session's save. */
  latestVersionNumber: number
  /** The version this edit session started from. */
  openedVersionNumber: number
  /** Decrypted plaintext of the racing (latest) version, once loaded. */
  latestText: string | null
  /** This session's current (unsaved) editor text. */
  localText: string
  /** True while a chosen resolution ('keep-both' / 'save-as-new-version') is
   *  uploading. Disables every action button so a slow upload can't be
   *  double-submitted (PR #103 review thread) — the real guard is
   *  file-editor.tsx's own `saving` check in handleConflictAction; this is
   *  the visible, immediate half of it. */
  saving?: boolean
  onAction: (action: ConflictAction) => void
  onCancel: () => void
}

export function ConflictDialog({
  latestVersionNumber,
  openedVersionNumber,
  latestText,
  localText,
  saving = false,
  onAction,
  onCancel,
}: ConflictDialogProps) {
  // 'show-differences' is itself one of the three offered actions (not just
  // a display toggle) — resolveConflictAction() deliberately returns null
  // for it: nothing uploads, the caller (this component) just reveals the
  // diff it already has the data for.
  const [differencesShown, setDifferencesShown] = useState(false)

  const diff: DiffLine[] | null = latestText !== null ? diffLines(latestText, localText) : null
  const changedLines = diff?.filter((l) => l.type !== 'same') ?? []
  const sideBySideRows = diff !== null ? toSideBySideRows(diff) : []

  function handleShowDifferences() {
    setDifferencesShown(true)
    onAction('show-differences')
  }

  return (
    <div
      className="absolute inset-0 z-40 flex items-center justify-center bg-black/40 p-4"
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="conflict-dialog-title"
        className="flex w-full max-w-[880px] max-h-[80%] flex-col overflow-hidden rounded-lg border border-line bg-paper shadow-3"
        data-testid="editor-conflict-dialog"
      >
        <div className="flex items-start gap-3 border-b border-line p-4">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-amber-bg text-amber-deep">
            <Icon name="shield" size={15} />
          </span>
          <div className="min-w-0">
            <h4 id="conflict-dialog-title" className="text-[14px] font-semibold text-ink">
              A newer version was saved elsewhere
            </h4>
            <p className="mt-1 text-[12px] leading-relaxed text-ink-3">
              Version {latestVersionNumber} · you started from version {openedVersionNumber}.
              Your changes are still here, and both versions stay in history.
            </p>
          </div>
        </div>

        {differencesShown && (
          <>
            <div className="grid grid-cols-2 border-b border-line bg-paper-2 text-[11px] font-medium text-ink-3">
              <span className="border-r border-line px-4 py-1.5">
                Version {latestVersionNumber} · saved elsewhere
              </span>
              <span className="px-4 py-1.5">Your edit · this browser · not saved</span>
            </div>

            <div
              className="flex-1 overflow-auto font-mono text-[11px] leading-[1.6]"
              data-testid="editor-conflict-diff"
            >
              {latestText === null ? (
                <div className="flex items-center gap-2 p-4 text-ink-3">
                  <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-line border-t-amber" />
                  Loading the other version…
                </div>
              ) : diff === null ? (
                <div className="p-4 text-ink-3">
                  These files are too large to show a line-by-line difference here
                  (over {MAX_DIFF_LINES.toLocaleString()} lines on one side). Choose an option
                  below — nothing is overwritten either way.
                </div>
              ) : changedLines.length === 0 ? (
                <div className="p-4 text-ink-3">
                  No line differences — the content is identical.
                </div>
              ) : (
                // Side-by-side (design/editor-1563.html screen 04): left =
                // the newer version saved elsewhere, right = this edit
                // session's own unsaved text, each with its OWN line
                // numbers — not a single unified +/- column (PR #103
                // review). A row whose other side has no corresponding
                // line (an add or a del) renders an empty filler cell there
                // so both columns stay aligned row-for-row.
                <div className="grid grid-cols-2" data-testid="editor-conflict-diff-split">
                  {sideBySideRows.map((row, i) => (
                    <Fragment key={i}>
                      <SideCellView cell={row.left} border="left" />
                      <SideCellView cell={row.right} border="right" />
                    </Fragment>
                  ))}
                </div>
              )}
            </div>
          </>
        )}

        {!differencesShown && (
          <div className="px-4 py-3 text-[12px] text-ink-3">
            {diff === null
              ? 'Both versions are too large to diff line-by-line here.'
              : changedLines.length === 0
                ? 'No line differences — the content is identical.'
                : `${changedLines.length} line${changedLines.length === 1 ? '' : 's'} differ between the two versions.`}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2 border-t border-line p-3">
          <span className="flex items-center gap-1.5 text-[11px] text-ink-3">
            <Icon name="lock" size={11} />
            Encrypted on this device before saving
          </span>
          <div className="ml-auto flex flex-wrap items-center gap-2">
            {saving && (
              <span
                className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-line border-t-amber"
                aria-hidden="true"
              />
            )}
            {!differencesShown && (
              <BBButton size="sm" variant="ghost" onClick={handleShowDifferences} disabled={saving}>
                Show differences
              </BBButton>
            )}
            <BBButton size="sm" variant="ghost" onClick={onCancel} disabled={saving}>
              Cancel
            </BBButton>
            <BBButton size="sm" onClick={() => onAction('keep-both')} disabled={saving}>
              Keep both
            </BBButton>
            <BBButton
              size="sm"
              variant="amber"
              onClick={() => onAction('save-as-new-version')}
              disabled={saving}
            >
              Save as version {latestVersionNumber + 1}
            </BBButton>
          </div>
        </div>
      </div>
    </div>
  )
}
