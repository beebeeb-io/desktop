/**
 * FloatingSelectionToolbar (task 1567) — B/I/U + colour + link, shown only
 * while `bbOffice.onSelectionChange()` reports non-empty text, positioned at
 * the pointer-up/double-click point on the canvas (lead decision 3, since
 * this engine build has no accessibility bridge to derive a real selection
 * screen rect — see bb-office-bridge.ts's `onSelectionChange` doc comment).
 * Hidden on typing or scroll by the caller (OfficeEditor owns that logic —
 * this component is purely positional/presentational).
 */

import { useRef } from 'react'
import { Icon } from '../../design'
import { IconTextColor } from './office-icons'
import { unoColorToHex, hexToUnoColor } from './char-formatting-controls'

export interface FloatingSelectionToolbarProps {
  /** Position in the HOST page's coordinate space (already translated from
   *  the iframe's own client coordinates by the caller). Null hides it. */
  position: { x: number; y: number } | null
  states: { bold: boolean; italic: boolean; underline: boolean }
  onBold: () => void
  onItalic: () => void
  onUnderline: () => void
  onLink: () => void
  /** CRITIQUE.md finding #7 (task 1567): the mockup's floating toolbar is
   *  B/I/U/color/link — five icons; the color icon was missing because #4's
   *  color dispatch didn't exist yet. `textColor` is the raw UNO CharColor
   *  (-1 = automatic) so this stays a pure/presentational component like the
   *  rest of this file, matching `onTextColor`'s own {@link ColorPickers}
   *  sibling in char-formatting-controls.tsx. */
  textColor?: number
  onTextColor?: (unoColor: number) => void
}

export function FloatingSelectionToolbar({
  position,
  states,
  onBold,
  onItalic,
  onUnderline,
  onLink,
  textColor = -1,
  onTextColor,
}: FloatingSelectionToolbarProps) {
  const colorInputRef = useRef<HTMLInputElement>(null)
  if (!position) return null
  return (
    <div
      className="absolute z-[2] flex h-9 items-center gap-0.5 rounded-lg border border-line bg-paper-2 px-1.5 shadow-2"
      style={{ left: position.x, top: position.y }}
      data-testid="office-selection-toolbar"
    >
      <button
        type="button"
        aria-label="Bold"
        aria-pressed={states.bold}
        onClick={onBold}
        className={`grid h-7 w-7 place-items-center rounded-md font-sans text-[13px] font-bold ${states.bold ? 'bg-amber-bg' : 'hover:bg-paper-3'}`}
      >
        B
      </button>
      <button
        type="button"
        aria-label="Italic"
        aria-pressed={states.italic}
        onClick={onItalic}
        className={`grid h-7 w-7 place-items-center rounded-md font-sans text-[13px] italic ${states.italic ? 'bg-amber-bg' : 'hover:bg-paper-3'}`}
      >
        I
      </button>
      <button
        type="button"
        aria-label="Underline"
        aria-pressed={states.underline}
        onClick={onUnderline}
        className={`grid h-7 w-7 place-items-center rounded-md font-sans text-[13px] underline ${states.underline ? 'bg-amber-bg' : 'hover:bg-paper-3'}`}
      >
        U
      </button>
      {onTextColor && (
        <div className="relative grid h-7 w-7 place-items-center rounded-md hover:bg-paper-3">
          <button
            type="button"
            aria-label="Text color"
            title="Text color"
            onClick={() => colorInputRef.current?.click()}
            className="grid h-full w-full place-items-center"
          >
            <IconTextColor size={14} barColor={unoColorToHex(textColor, '#1a1a1a')} />
          </button>
          <input
            ref={colorInputRef}
            type="color"
            aria-hidden="true"
            tabIndex={-1}
            data-testid="selection-toolbar-text-color-input"
            value={unoColorToHex(textColor, '#1a1a1a')}
            onChange={(e) => onTextColor(hexToUnoColor(e.target.value))}
            className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          />
        </div>
      )}
      <div className="mx-0.5 h-[18px] w-px bg-line" />
      <button type="button" aria-label="Insert link" onClick={onLink} className="grid h-7 w-7 place-items-center rounded-md hover:bg-paper-3">
        <Icon name="link" size={14} />
      </button>
    </div>
  )
}
