import type { UploadReviewChoice, VersionConflictEntry } from '../desktopApi'

export const UPLOAD_REVIEW_LABELS: Record<UploadReviewChoice, string> = {
  keep_both: 'Keep both',
  keep_mine: 'Keep mine',
  discard: 'Discard',
}

const UPLOAD_REVIEW_HINTS: Record<UploadReviewChoice, string> = {
  keep_both: 'Upload your edit as a separate copy named after this device; the file follows the newer server version.',
  keep_mine: 'Upload your edit as the newest version. The other version stays in the file’s version history.',
  discard: 'Delete your queued edit and follow the server version. We can’t recover the discarded edit.',
}

function isUploadReviewChoice(value: string): value is UploadReviewChoice {
  return value === 'keep_both' || value === 'keep_mine' || value === 'discard'
}

/** The in-place choices the daemon offers for a review entry (server-driven). */
export function uploadReviewChoices(entry: VersionConflictEntry): UploadReviewChoice[] {
  if (entry.action !== 'review_upload' || !entry.op_id) return []
  return (entry.resolutions ?? []).filter(isUploadReviewChoice)
}

export function ReviewEntryActions({
  entry,
  busy,
  confirmingDiscard,
  onOpenConflict,
  onResolve,
}: {
  entry: VersionConflictEntry
  /** The choice currently being applied for this entry, if any. */
  busy: UploadReviewChoice | null
  /** Discard deletes local bytes, so it takes a second, explicit click. */
  confirmingDiscard: boolean
  onOpenConflict: (entry: VersionConflictEntry) => void
  onResolve: (entry: VersionConflictEntry, choice: UploadReviewChoice) => void
}) {
  if (entry.action === 'open_conflict') {
    return (
      <button className="button" onClick={() => onOpenConflict(entry)}>
        Review
      </button>
    )
  }

  const choices = uploadReviewChoices(entry)
  if (choices.length === 0) return null

  return (
    <div className="button-row" role="group" aria-label={`Resolve ${entry.file_name}`}>
      {choices.map((choice) => {
        const confirming = choice === 'discard' && confirmingDiscard
        const className =
          choice === 'keep_both' ? 'button primary' : choice === 'discard' ? 'button danger' : 'button'
        return (
          <button
            key={choice}
            className={className}
            disabled={busy !== null}
            title={UPLOAD_REVIEW_HINTS[choice]}
            onClick={() => onResolve(entry, choice)}
          >
            {busy === choice ? 'Working' : confirming ? 'Confirm discard' : UPLOAD_REVIEW_LABELS[choice]}
          </button>
        )
      })}
    </div>
  )
}
