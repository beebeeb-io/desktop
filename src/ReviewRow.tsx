/**
 * One row of Review (task 1683 slice 4; spec §7.6, hi-fi section 3): the mono kind tag, the file's name, `folder · when ·
 * detail`, and at most one button. Review draws these, and so does Home's "Needs your attention", so a row reads the same
 * in both. Presentational: it holds no state and calls `onAction(entry, action)`, which `reviewActions.ts` answers.
 */
import type { VersionConflictEntry } from './desktopApi'
import { Btn } from './macSettingsParts'
import { reviewKindLabel, reviewRowAction, reviewRowMeta, type ReviewRowAction } from './reviewModel'

export function ReviewEntryRow({
  entry,
  now,
  onAction,
}: {
  entry: VersionConflictEntry
  now: Date
  onAction: (entry: VersionConflictEntry, action: ReviewRowAction) => void
}) {
  const action = reviewRowAction(entry)
  const meta = reviewRowMeta(entry, now)
  return (
    <div className="mw-row" data-row="review">
      <span className="mw-ktag">{reviewKindLabel(entry.kind)}</span>
      <div className="mw-row-text">
        <div className="mw-row-title">{entry.file_name}</div>
        {meta ? <div className="mw-row-hint">{meta}</div> : null}
      </div>
      {action ? <Btn onClick={() => onAction(entry, action)}>{action.label}</Btn> : null}
    </div>
  )
}
