/**
 * CalcStatusExtra (task 1567, Calc lane) -- "Sum €45,200 · Average €11,300 ·
 * Count 4" segment of the status bar, matching
 * design/office-editor-shots/calc-*.png. Passed into `OfficeStatusBar`'s new
 * `extra` slot (the one status-bar registration hook this lane's brief
 * allows) by `office-editor.tsx` when `officeApp === 'calc'`.
 *
 * Renders nothing (not "Sum —") when there is no selection to summarize yet,
 * or when the underlying clipboard-based read failed -- see
 * `../../lib/office/use-calc-selection-stats.ts`'s header for the full,
 * verified explanation of why this can't be a simpler live engine readout.
 */
import type { CalcSelectionStats } from '../lib/office/calc-selection-stats'
import { formatCalcCurrency, formatCalcNumber } from '../lib/office/calc-selection-stats'

export interface CalcStatusExtraProps {
  stats: CalcSelectionStats | null
}

export function CalcStatusExtra({ stats }: CalcStatusExtraProps) {
  if (!stats) return null
  const fmt = stats.hadCurrency ? formatCalcCurrency : formatCalcNumber
  return (
    <>
      <span className="min-w-0 truncate" data-testid="calc-status-sum">
        Sum {fmt(stats.sum)}
      </span>
      {/* Task 1585: at phone width only Sum fits next to the save state. */}
      <span className="hidden sm:inline" data-testid="calc-status-average">
        Average {fmt(stats.average)}
      </span>
      <span className="hidden sm:inline" data-testid="calc-status-count">
        Count {stats.count}
      </span>
    </>
  )
}
