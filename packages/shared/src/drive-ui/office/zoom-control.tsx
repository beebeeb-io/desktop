/**
 * ZoomControl (task 1567) — floating pill, bottom-right of the canvas.
 *
 * Design nit from the brief: the mockup's zoom pill overlaps the status bar
 * at some viewport heights. Fixed here by anchoring it `bottom-[38px]`
 * (status bar height 30px + 8px clearance) INSIDE the canvas area rather than
 * the mockup's `bottom:16px` against the whole frame — the status bar is a
 * sibling flex row below the canvas, not an overlay on top of it, so this is
 * a one-line position fix, not a z-index hack.
 */

import { IconMinus } from './office-icons'
import { Icon } from '../../design'

export interface ZoomControlProps {
  percent: number
  onChange: (percent: number) => void
  min?: number
  max?: number
  step?: number
}

export function ZoomControl({ percent, onChange, min = 50, max = 200, step = 10 }: ZoomControlProps) {
  return (
    <div
      className="absolute bottom-[38px] right-4 z-[3] flex h-[30px] items-center gap-2 rounded-full border border-line bg-paper-2 px-2 font-mono text-[11px] text-ink-3 shadow-2"
      data-testid="office-zoom-control"
    >
      <button
        type="button"
        aria-label="Zoom out"
        onClick={() => onChange(Math.max(min, percent - step))}
        className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-paper-3 text-ink"
      >
        <IconMinus size={10} />
      </button>
      {/*
        CRITIQUE.md finding #6 (task 1567): the mockup's zoom pill has a real
        draggable track+thumb (design/office-editor.html's `.zoom .track` +
        `.track i`), matching Word's/Docs' own zoom sliders — what shipped
        before this fix was click-to-step only. A native <input type="range">
        keeps the existing stepper buttons AND `min`/`max`/`step` (already
        real props, previously only used for the button math) for keyboard/
        accessibility, while adding drag. Styled to match the mockup's own
        50px track / 3px height / 9px thumb exactly (not a browser default
        slider) via the `::-webkit-slider-*` / `::-moz-range-*` pseudo-
        elements — Tailwind 4's arbitrary-variant syntax reaches both without
        a dependency.
      */}
      <input
        type="range"
        aria-label="Zoom level"
        data-testid="office-zoom-slider"
        min={min}
        max={max}
        step={step}
        value={percent}
        onChange={(e) => onChange(Number(e.target.value))}
        className="h-[9px] w-[50px] shrink-0 cursor-pointer appearance-none bg-transparent
          [&::-webkit-slider-runnable-track]:h-[3px] [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-line
          [&::-webkit-slider-thumb]:mt-[-3px] [&::-webkit-slider-thumb]:h-[9px] [&::-webkit-slider-thumb]:w-[9px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-ink
          [&::-moz-range-track]:h-[3px] [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-line
          [&::-moz-range-thumb]:h-[9px] [&::-moz-range-thumb]:w-[9px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-ink"
      />
      <button
        type="button"
        aria-label="Zoom in"
        onClick={() => onChange(Math.min(max, percent + step))}
        className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-paper-3 text-ink"
      >
        <Icon name="plus" size={10} />
      </button>
      <span className="w-9 text-center tabular-nums">{percent}%</span>
    </div>
  )
}
