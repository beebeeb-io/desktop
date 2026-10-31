/**
 * Home's three additions as the window draws them (task 1683 slice 4, the Home add-on; spec §7.6 Home, hi-fi section 3):
 * the counts line inside the status card, "Needs your attention" (Review's own rows, then the kept rows), and "Recent
 * activity". Presentational and hook-free; `homeModel.ts` decides what they say and `MacApp.tsx` `HomePane` supplies it.
 * A section with nothing to say draws nothing: no placeholder row.
 */
import type { ReactNode } from 'react'
import type { VersionConflictEntry } from './desktopApi'
import type { HomeActivityRow, HomeCountPart } from './homeModel'
import { Btn, SettingsGroup, SettingsIcon } from './macSettingsParts'
import { ReviewEntryRow } from './ReviewRow'
import type { ReviewRowAction } from './reviewModel'

/** "On this Mac 1,204 files · Online-only 8,311 · Kept on this Mac 3 folders": a label and a bold value each. */
export function HomeCounts({ parts }: { parts: HomeCountPart[] | null }) {
  if (parts === null || parts.length === 0) return null
  return (
    <div className="mw-row" data-row="counts">
      <div className="mw-counts">
        {parts.map((part) => (
          <span key={part.label}>
            {part.label} <b>{part.value}</b>
          </span>
        ))}
      </div>
    </div>
  )
}

/**
 * Review's top entries, then whatever the caller adds (the kept rows, §8.3), and one quiet button for the rest. With
 * nothing of its own and nothing added the group is not drawn.
 */
export function HomeAttention({
  entries,
  more,
  now,
  onAction,
  onOpenReview,
  children,
}: {
  entries: VersionConflictEntry[]
  more: number
  now: Date
  onAction: (entry: VersionConflictEntry, action: ReviewRowAction) => void
  onOpenReview: () => void
  children?: ReactNode
}) {
  if (entries.length === 0 && !children) return null
  return (
    <SettingsGroup title="Needs your attention">
      {entries.map((entry) => (
        <ReviewEntryRow key={entry.id} entry={entry} now={now} onAction={onAction} />
      ))}
      {more > 0 ? (
        <div className="mw-row">
          <Btn onClick={onOpenReview}>{`${more} more in Review`}</Btn>
        </div>
      ) : null}
      {children}
    </SettingsGroup>
  )
}

/** The popover's rows, more of them: a direction glyph, the name, and what happened to it and when. */
export function HomeActivity({ rows }: { rows: HomeActivityRow[] }) {
  if (rows.length === 0) return null
  return (
    <SettingsGroup title="Recent activity">
      <div role="list">
        {rows.map((row) => (
          <div className="mw-row" role="listitem" aria-label={row.label} title={row.path} key={row.id}>
            <span className={row.tone === 'alert' ? 'mw-tile mw-tile--alert' : 'mw-tile'} aria-hidden="true">
              <SettingsIcon name={row.icon === 'up' ? 'arrowUp' : row.icon === 'down' ? 'arrowDown' : 'alert'} size={15} />
            </span>
            <div className="mw-row-text">
              <div className="mw-row-title">{row.name}</div>
              <div className="mw-row-hint">{row.line}</div>
            </div>
          </div>
        ))}
      </div>
    </SettingsGroup>
  )
}
