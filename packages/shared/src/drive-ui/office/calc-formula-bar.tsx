/**
 * CalcFormulaBar (task 1567, Calc lane) -- design/office-editor-shots/
 * calc-*.png's "C2:C5 · ƒx · =SUM(C2:C5)" row, in JetBrains Mono.
 *
 * DEVIATION from the mockup, flagged not silently dropped (matching this
 * task's own established convention for engine gaps): the mockup's ref box
 * and value both PASSIVELY reflect the current selection. There is no bridge
 * primitive to read the active cell's address or formula text (see
 * `../../lib/office/use-calc-selection-stats.ts`'s header for the full
 * verified gap list), so this can't be a live readout. What IS real: typing
 * a cell or range reference and pressing Enter navigates there via
 * `.uno:GoToCell` -- the same underlying mechanism a Name Box uses in every
 * spreadsheet app, verified against the real engine artifact (a `ToPoint`
 * arg of "A1:A3" really does select that range -- confirmed via a
 * `.uno:Copy` + clipboard read showing all three cells' values). The value
 * slot shows this box's own status rather than a formula it cannot read.
 */
import { useState } from 'react'
import type { OfficeBridge } from '../lib/office/bb-office-bridge'

const CELL_REF_RE = /^\$?[A-Za-z]{1,3}\$?[0-9]{1,7}(:\$?[A-Za-z]{1,3}\$?[0-9]{1,7})?$/

export interface CalcFormulaBarProps {
  bridge: OfficeBridge | null
  /** Called after a successful navigation -- office-editor.tsx uses this to
   *  hand DOM keyboard focus back to the engine canvas (this box is our own
   *  plain HTML `<input>`, so focus never moves there on its own the way a
   *  native spreadsheet's Name Box would), so a user can start typing into
   *  the cell they just jumped to right away. */
  onNavigated?: () => void
}

export function CalcFormulaBar({ bridge, onNavigated }: CalcFormulaBarProps) {
  const [value, setValue] = useState('')
  const [status, setStatus] = useState<'idle' | 'ok' | 'error'>('idle')

  function goToRef() {
    const target = value.trim().toUpperCase()
    if (!CELL_REF_RE.test(target)) {
      setStatus('error')
      return
    }
    bridge
      ?.dispatch('.uno:GoToCell', [{ name: 'ToPoint', value: target }])
      .then(() => {
        setStatus('ok')
        onNavigated?.()
      })
      .catch(() => setStatus('error'))
  }

  return (
    <div className="flex h-[34px] shrink-0 items-center gap-2.5 border-b border-line bg-paper-2 px-4 font-mono text-[12.5px]" data-testid="calc-formula-bar">
      <input
        value={value}
        onChange={(e) => {
          setValue(e.target.value)
          setStatus('idle')
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            goToRef()
          }
        }}
        placeholder="Go to cell"
        aria-label="Go to cell or range"
        title="Type a cell or range (e.g. B5 or A1:A3) and press Enter to go there"
        data-testid="calc-ref-box"
        className={`h-[23px] w-[92px] shrink-0 rounded-md border bg-paper px-2 text-center text-ink outline-none focus:border-amber ${
          status === 'error' ? 'border-red-border' : 'border-line'
        }`}
      />
      <span className="shrink-0 font-semibold text-ink-3">ƒx</span>
      <span className="min-w-0 flex-1 truncate text-ink-4" data-testid="calc-formula-bar-status">
        {status === 'error' ? 'Not a valid cell or range' : status === 'ok' ? `Went to ${value.trim().toUpperCase()}` : ''}
      </span>
    </div>
  )
}
