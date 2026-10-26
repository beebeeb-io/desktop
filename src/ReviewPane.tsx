/**
 * The Review section of the Beebeeb window (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.6, §10.2 rows 11, 13, 16, 17; hi-fi section 3). It replaces "Versions & conflicts", the per-conflict windows and
 * the version-history modal: one row per entry of the Review feed (`list_version_conflict_center`), each with a mono
 * kind tag, the file's name, when it happened and Rust's detail line, and at most one button.
 *
 * What a button does: "Decide…" opens the conflict sheet over this section; "Add storage" and "Manage plan" open the
 * billing page in the browser; "Sign in again" starts the in-place sign-in (R8; nothing is cleared). Every other kind has no button: a
 * fake one would be worse than none (task 1546). "Version history…" is the window's toolbar button; this pane owns the
 * sheet it opens (`historyOpen`), so it is one sheet over Review whether or not anything needs a decision.
 *
 * The pane reads the feed it is given (`reviewFeed.ts`, shared with the sidebar's badge) and asks its owner to read it
 * again after a resolved conflict (`onChanged`), so the row leaves. A vault that is locked never reaches this pane:
 * the window shows the locked card instead (`LockedPane.tsx`).
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported here, so
 * `tests/reviewPane.test.tsx` can execute them with the component harness.
 */
import { useState } from 'react'
import { command, forceReauth, type VersionConflictEntry } from './desktopApi'
import { ConflictSheet } from './ConflictSheet'
import { Btn, Note } from './macSettingsParts'
import type { ReviewFeedState } from './reviewFeed'
import { makeReviewActions } from './reviewActions'
import { REVIEW_EMPTY, REVIEW_INTRO, REVIEW_SOME_UNREADABLE, reviewMoreLine } from './reviewModel'
import { ReviewEntryRow } from './ReviewRow'
import { VersionHistorySheet } from './VersionHistorySheet'
import { useToast } from './windows/ui'

export function ReviewPane({
  feed,
  historyOpen,
  onHistoryClose,
  onChanged,
}: {
  feed: ReviewFeedState
  historyOpen: boolean
  onHistoryClose: () => void
  onChanged: () => void
}) {
  const { showToast } = useToast()
  const [deciding, setDeciding] = useState<VersionConflictEntry | null>(null)

  // What a row's button does is one function for Review and Home (`reviewActions.ts`).
  const act = makeReviewActions({
    showToast, onDecide: setDeciding, forceReauth, onChanged,
    openUrl: (url) => command<void>('plugin:opener|open_url', { url }),
    dismiss: (entryId) => command<void>('dismiss_review_entry', { entryId }),
  })

  const now = new Date()
  let body
  if (feed.status === 'loading') {
    body = (
      <div className="ms-loading" role="status">
        Loading…
      </div>
    )
  } else if (feed.status === 'failed') {
    body = (
      <Note kind="alert" surface="review-load" title="Couldn’t load Review" actions={<Btn onClick={onChanged}>Try again</Btn>}>
        {feed.reason}
      </Note>
    )
  } else {
    // What waits and is neither drawn nor accounted for: the count includes the entries that did not read (the note below says
    // those), so they are not "more" as well.
    const more = reviewMoreLine(feed.count - feed.rejected, feed.entries.length)
    // "Nothing needs your attention" is said only when nothing is waiting AND nothing was left out of this list.
    const nothing = feed.entries.length === 0 && feed.rejected === 0 && more === null
    body = (
      <>
        <p className="mw-intro">{REVIEW_INTRO}</p>
        {nothing ? (
          <div className="mw-card">
            <div className="mw-empty">{REVIEW_EMPTY}</div>
          </div>
        ) : feed.entries.length > 0 ? (
          <div className="mw-card">
            {feed.entries.map((entry) => (
              <ReviewEntryRow key={entry.id} entry={entry} now={now} onAction={act} />
            ))}
          </div>
        ) : null}
        {more ? (
          <p className="mw-intro" data-note="more">
            {more}
          </p>
        ) : null}
        {feed.rejected > 0 ? (
          <p className="mw-intro" data-note="unreadable" role="status">
            {REVIEW_SOME_UNREADABLE}
          </p>
        ) : null}
      </>
    )
  }

  return (
    <div className="mw-content">
      {body}
      {deciding ? <ConflictSheet entry={deciding} onClose={() => setDeciding(null)} onResolved={onChanged} /> : null}
      {historyOpen ? <VersionHistorySheet onClose={onHistoryClose} /> : null}
    </div>
  )
}
