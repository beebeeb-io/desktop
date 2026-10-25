/**
 * The conflict sheet (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6 "Conflict sheet
 * (520 pt, from ConflictWindow.tsx)", §10.2 row 35; hi-fi section 3). It replaces the per-conflict window on a Mac:
 * nothing opens a window for a conflict any more, and Review's "Decide…" opens this sheet over the Beebeeb window.
 *
 * Derived from `ConflictWindow.tsx`, which Windows and Linux keep unchanged: the same two commands
 * (`conflict_content_preview` for both sides' real content, `resolve_conflict` for the choice) and the same line diff
 * (`diffLines`), in the words and layout of the hi-fi. The sheet is the window's `Modal`, so it counts as an open sheet
 * (§8.2: nothing navigates under it), traps focus, closes on Esc, and OPENS ON ITS CLOSE BUTTON: Enter on open is
 * Cancel, never one of the three choices (§11, the 10-02 destructive-Enter rule applied to a choice with consequences).
 *
 * While a choice is being applied nothing can close the sheet: closing would hide the outcome of a change already
 * under way. On success the sheet tells its owner (the row leaves), says "Conflict resolved" once, and closes. On
 * failure it stays open, says why once, and the choices work again.
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported here, so
 * `tests/conflictSheet.test.tsx` can execute them with the component harness.
 */
import { useEffect, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  conflictContentPreview,
  type ConflictContentPreview,
  type ConflictContentSide,
  type VersionConflictEntry,
} from './desktopApi'
import { diffLines, DIFF_MAX_OPS, type DiffOp } from './diffLines'
import { Btn } from './macSettingsParts'
import {
  CONFLICT_CHOICES,
  CONFLICT_RESOLVED_TOAST,
  CONFLICT_SHEET_INTRO,
  CONFLICT_SIDE_LABELS,
  conflictSheetTitle,
  type ConflictChoice,
} from './reviewModel'
import { formatStorageSize } from './storageFormat'
import { Modal, useToast } from './windows/ui'

/** One line of a side: `-` what only this Mac has, `+` what only the other device has, a blank for shared lines. */
function ConflictLine({ line, type }: { line: string; type: 'add' | 'remove' | 'same' }) {
  const prefix = type === 'add' ? '+' : type === 'remove' ? '-' : ' '
  return (
    <div className={`mw-diff-line mw-diff-line--${type}`} data-line={type}>
      {prefix} {line}
    </div>
  )
}

/**
 * One side: the diff lines that belong to it (its own and the shared ones), or the honest reason it cannot be shown, or a
 * binary file's size. Never filler text.
 */
function ConflictSide({
  which,
  label,
  side,
  loading,
  isText,
  ops,
  only,
}: {
  which: 'local' | 'remote'
  label: string
  side: ConflictContentSide | null
  loading: boolean
  isText: boolean
  ops: DiffOp[] | null
  only: 'remove' | 'add'
}) {
  let body
  if (loading) {
    body = <div className="mw-compare-note">Loading…</div>
  } else if (side?.unavailable_reason) {
    body = <div className="mw-compare-note">{side.unavailable_reason}</div>
  } else if (isText && side?.text != null) {
    body = ops ? (
      ops
        .filter((op) => op.type === 'same' || op.type === only)
        .map((op, index) => <ConflictLine key={index} line={op.line} type={op.type} />)
    ) : (
      // Too long to compare line by line: still the real content, just unhighlighted.
      <ConflictLine line={side.text} type="same" />
    )
  } else if (!isText && side) {
    const size = typeof side.size_bytes === 'number' ? ` · ${formatStorageSize(side.size_bytes)}` : ''
    body = <div className="mw-compare-note">{`Binary file${size}`}</div>
  } else {
    body = <div className="mw-compare-note">Content unavailable.</div>
  }
  return (
    <div className="mw-compare-side" data-side={which}>
      <b>{label}</b>
      <div className="mw-compare-body">{body}</div>
    </div>
  )
}

export function ConflictSheet({
  entry,
  onClose,
  onResolved,
}: {
  entry: VersionConflictEntry
  onClose: () => void
  onResolved: () => void
}) {
  const { showToast } = useToast()
  const [preview, setPreview] = useState<ConflictContentPreview | null>(null)
  const [previewLoading, setPreviewLoading] = useState(true)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    setPreviewLoading(true)
    void conflictContentPreview(entry.file_id).then((result) => {
      if (cancelled) return
      setPreviewLoading(false)
      if (result.ok) setPreview(result.value)
      else setPreviewError(result.unsupported ? commandUnavailableLabel('conflict_content_preview') : result.reason)
    })
    return () => {
      cancelled = true
    }
  }, [entry.file_id])

  const resolve = async (choice: ConflictChoice) => {
    setBusy(true)
    const result = await command<void>('resolve_conflict', { fileId: entry.file_id, choice })
    if (!result.ok) {
      setBusy(false)
      showToast({
        variant: 'error',
        title: 'Couldn’t resolve this conflict',
        message: result.unsupported ? commandUnavailableLabel('resolve_conflict') : result.reason,
      })
      return
    }
    showToast({ ...CONFLICT_RESOLVED_TOAST })
    onResolved()
    onClose()
  }

  const isText = preview?.is_text === true
  const both = preview?.local.text != null && preview?.remote.text != null
  const diff = both ? diffLines(preview.local.text as string, preview.remote.text as string) : null
  const ops = diff?.ops ?? null
  const title = conflictSheetTitle(entry.file_name)

  return (
    <Modal
      open
      onClose={busy ? () => {} : onClose}
      title={title}
      ariaLabel={title}
      maxWidth={520}
      footer={
        <div className="ms-sheet-footer">
          {CONFLICT_CHOICES.map((item) => (
            <Btn key={item.choice} primary={item.primary} disabled={busy} onClick={() => void resolve(item.choice)}>
              {item.label}
            </Btn>
          ))}
        </div>
      }
    >
      <p className="ms-sheet-copy">{CONFLICT_SHEET_INTRO}</p>
      {previewLoading ? (
        <div className="mw-sheet-note" role="status">
          Loading both versions…
        </div>
      ) : null}
      {previewError ? (
        <div className="mw-sheet-note">{`Couldn’t load either version’s content: ${previewError}. You can still choose a version below.`}</div>
      ) : null}
      {!previewLoading && isText && both && ops === null ? (
        <div className="mw-sheet-note">Both files are too large to compare line by line. Showing the full text without highlighting.</div>
      ) : null}
      {!previewLoading && diff?.truncated ? (
        <div className="mw-sheet-note">{`Diff truncated. Showing the first ${DIFF_MAX_OPS.toLocaleString()} changes.`}</div>
      ) : null}
      <div className="mw-compare">
        <ConflictSide which="local" label={CONFLICT_SIDE_LABELS.local} side={preview?.local ?? null} loading={previewLoading} isText={isText} ops={ops} only="remove" />
        <ConflictSide which="remote" label={CONFLICT_SIDE_LABELS.remote} side={preview?.remote ?? null} loading={previewLoading} isText={isText} ops={ops} only="add" />
      </div>
    </Modal>
  )
}
