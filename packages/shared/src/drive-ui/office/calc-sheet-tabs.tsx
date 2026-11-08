/**
 * CalcSheetTabs (task 1567, Calc lane) -- house-styled sheet tab bar,
 * matching design/office-editor-shots/calc-*.png's "Q3 Budget · Notes ·
 * Forecast · +" row.
 *
 * REAL, CONFIRMED ENGINE GAP (not this lane's bug): probed against the
 * actual rebuilt engine artifact (repos/office/evidence/artifacts/
 * emscripten, phase4-2026-09-27) across 3 separate real investigations --
 *   1. `.uno:JumpToTable` dispatch (sheet switching) always resolves
 *      `{dispatched:true}` but never actually changes the active sheet --
 *      tried 8 distinct argument shapes (numeric 0/1/2-based index, the
 *      sheet's own name as a string, and 4 different argument-name
 *      spellings: Table/Item/Index/Name/Value), verified each time by
 *      reading BOTH `.uno:Name`'s state AND the real content of cell A1 (via
 *      GoToCell+Copy+clipboard) -- neither ever changed.
 *   2. Every plausible "insert a new sheet" command name (`.uno:InsertTable`,
 *      `.uno:Insert Sheet`, `.uno:InsertSheet`, `.uno:Insert$Table`) reports
 *      "no dispatch handler" -- not registered in this build at all.
 *   3. There is no bridge op to enumerate sheet names/count either (the
 *      worker's full op table has no such thing -- see
 *      use-calc-selection-stats.ts's header for the complete list).
 * `.uno:RenameTable` DOES work, verified round-trip (dispatch, then
 * `.uno:Name`'s state reads back the new name).
 *
 * Given this, honesty over a fake multi-tab strip: this renders the ONE
 * sheet this bridge can actually see (its real, live name, via
 * `.uno:Name`'s onState) with real, working rename, and an "Add sheet"
 * button that is visibly present (matching the mockup's chrome) but
 * disabled with a truthful tooltip -- never silently wired to a dispatch
 * that does nothing. Closing this gap needs a new bb-office-worker.js op
 * (e.g. `activateSheet(index)` / `getSheets()`), which is a different
 * repo's shared runtime bridge, out of this lane's "new files only" scope --
 * flagged in this lane's dated task note for the engine lane to pick up.
 */
import { useEffect, useState } from 'react'
import type { OfficeBridge } from '../lib/office/bb-office-bridge'
import { IconPlusSmall } from './calc-icons'

export interface CalcSheetTabsProps {
  bridge: OfficeBridge | null
  docReady: boolean
}

export function CalcSheetTabs({ bridge, docReady }: CalcSheetTabsProps) {
  const [name, setName] = useState<string | null>(null)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')

  useEffect(() => {
    if (!docReady || !bridge) return
    let unsub: (() => void) | undefined
    let cancelled = false
    bridge
      .onState('.uno:Name', (s) => {
        if (typeof s.state === 'string') setName(s.state)
      })
      .then((u) => {
        if (cancelled) unsub?.()
        else unsub = u
      })
      .catch(() => {})
    return () => {
      cancelled = true
      unsub?.()
    }
  }, [docReady, bridge])

  function commitRename() {
    const trimmed = draft.trim()
    setEditing(false)
    if (!trimmed || trimmed === name || !bridge) return
    bridge.dispatch('.uno:RenameTable', [{ name: 'Name', value: trimmed }]).catch(() => {})
    setName(trimmed) // optimistic -- .uno:Name's own onState confirms it moments later
  }

  return (
    <div className="flex h-[30px] shrink-0 items-center gap-0.5 border-t border-line bg-paper-2 px-2.5" data-testid="calc-sheet-tabs">
      {name !== null &&
        (editing ? (
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              if (e.key === 'Enter') commitRename()
              if (e.key === 'Escape') setEditing(false)
            }}
            data-testid="calc-sheet-tab-rename-input"
            className="h-[22px] rounded border border-amber bg-paper px-2 text-[12.5px] text-ink outline-none"
          />
        ) : (
          <button
            type="button"
            onDoubleClick={() => {
              setDraft(name)
              setEditing(true)
            }}
            title="Double-click to rename this sheet"
            data-testid="calc-sheet-tab-active"
            className="relative flex h-full items-center whitespace-nowrap px-3 text-[12.5px] font-semibold text-ink"
          >
            {name}
            <span className="absolute inset-x-1.5 top-0 h-[2px] rounded-b-sm bg-amber" />
          </button>
        ))}
      <button
        type="button"
        disabled
        title="Adding sheets isn't available in this editor yet"
        aria-label="Add sheet"
        data-testid="calc-sheet-tab-add"
        className="grid h-[22px] w-[22px] shrink-0 place-items-center rounded text-ink-4 opacity-40"
      >
        <IconPlusSmall size={12} />
      </button>
    </div>
  )
}
