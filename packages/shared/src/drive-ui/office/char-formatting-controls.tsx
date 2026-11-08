/**
 * CharFormattingControls (task 1567, CRITIQUE.md finding #4) — font family,
 * font size, text color and highlight color, the four Home-tab controls the
 * approved mockup (design/office-editor.html) shows and that were entirely
 * absent from the shipped ribbon: `WRITER_HOME_COMMANDS` had no entry for any
 * of them, and there was no UI path to change a font/size/color at all since
 * LibreOffice's own native toolbars are deliberately hidden (PLAN.md's hybrid
 * architecture).
 *
 * All four dispatch through the SAME generic `bbOffice.dispatch()` the rest
 * of the ribbon already uses — verified empirically against the real engine
 * (2026-09-27) that these commands/argument names/types are correct (not
 * guessed): `.uno:CharFontName` (`CharFontName.FamilyName`, string),
 * `.uno:FontHeight` (`FontHeight.Height`, float or long — both accepted),
 * `.uno:Color` (`Color`, long RGB), `.uno:CharBackColor` (`CharBackColor`,
 * long RGB). No new bridge/engine primitive was needed; the critique's own
 * fix suggestion ("this needs real bridge primitives... following the same
 * dispatch()/onState() pattern already proven for Bold/Italic") turned out to
 * already exist generically — it just needed the right command names.
 *
 * Font list is a CURATED set, not an exhaustive enumeration: a real UNO font-
 * enumeration probe (`com.sun.star.awt.Toolkit` → `createScreenCompatibleDevice`
 * → `FontDescriptors`) was attempted against the real engine and failed
 * (`XPropertySet.query()` on the returned device came back null in this
 * build) — stated here rather than silently shipping a guessed-complete list.
 * The three Liberation families are LibreOffice's own bundled, always-present
 * metric-compatible fonts; Arial/Times New Roman/Courier New/Calibri/Georgia
 * are included because LO's font-substitution table maps them to a rendered
 * equivalent by name even when the literal font isn't installed (also a
 * real, documented LO behavior, not a guess) — so every entry here renders
 * something correct, even though this is not the complete list an exhaustive
 * enumeration would show.
 */

import { useRef } from 'react'
import { IconChevronDown, IconTextColor, IconHighlighter } from './office-icons'

export const CURATED_FONT_FAMILIES = [
  'Liberation Sans',
  'Liberation Serif',
  'Liberation Mono',
  'Arial',
  'Times New Roman',
  'Courier New',
  'Calibri',
  'Georgia',
] as const

/** Word's own classic point-size list (its default toolbar picker), reused
 *  here rather than invented from scratch. */
export const FONT_SIZES = [8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72] as const

/** UNO's CharColor/CharBackColor are signed 32-bit RGB ints, or -1 for
 *  "automatic" (no explicit color set) — never negative otherwise in
 *  practice, so treating any negative/non-finite value as "automatic" is
 *  safe. `<input type="color">` needs a concrete #rrggbb, so "automatic"
 *  falls back to a sane default per swatch (ink black for text, transparent-
 *  looking white for highlight) rather than crashing on -1. */
export function unoColorToHex(value: number, fallback: string): string {
  if (!Number.isFinite(value) || value < 0) return fallback
  const clamped = Math.min(0xffffff, Math.max(0, Math.round(value)))
  return '#' + clamped.toString(16).padStart(6, '0')
}

export function hexToUnoColor(hex: string): number {
  return parseInt(hex.replace('#', ''), 16)
}

export interface CharFormattingState {
  /** Current font family name, or "" while unknown/mixed. */
  fontName: string
  /** Current point size, or null while unknown/mixed. */
  fontHeight: number | null
  /** Raw UNO CharColor (-1 = automatic). */
  textColor: number
  /** Raw UNO CharBackColor (-1 = none). */
  highlightColor: number
}

export interface FontPickersProps {
  fontName: string
  fontHeight: number | null
  onFontName: (name: string) => void
  onFontHeight: (points: number) => void
}

export interface ColorPickersProps {
  textColor: number
  highlightColor: number
  onTextColor: (unoColor: number) => void
  onHighlightColor: (unoColor: number) => void
}

/** Shared select styling matching the ribbon's existing "Normal text" pill
 *  (ribbon.tsx) so this doesn't introduce a second visual language for the
 *  same kind of control. */
const selectClass =
  'h-[30px] shrink-0 appearance-none rounded-md border border-line bg-paper px-2 pr-6 text-[12.5px] text-ink outline-none hover:bg-paper-3 focus-visible:ring-1 focus-visible:ring-amber'

function SelectChevron() {
  return <IconChevronDown size={10} className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-ink-3" />
}

/** Placed right after the paragraph-style pill, matching the mockup's own
 *  described order: "the style picker, font + size, B/I/U/S, ..."
 *  (design/office-editor.html's Home-tab caption). */
export function FontPickers({ fontName, fontHeight, onFontName, onFontHeight }: FontPickersProps) {
  return (
    <div className="flex shrink-0 items-center gap-1.5" data-testid="office-font-pickers">
      <div className="relative">
        <select
          aria-label="Font family"
          data-testid="ribbon-font-family"
          value={CURATED_FONT_FAMILIES.includes(fontName as (typeof CURATED_FONT_FAMILIES)[number]) ? fontName : ''}
          onChange={(e) => onFontName(e.target.value)}
          className={`${selectClass} w-[132px]`}
        >
          {!CURATED_FONT_FAMILIES.includes(fontName as (typeof CURATED_FONT_FAMILIES)[number]) && (
            <option value="" disabled>
              {fontName || 'Font'}
            </option>
          )}
          {CURATED_FONT_FAMILIES.map((f) => (
            <option key={f} value={f} style={{ fontFamily: f }}>
              {f}
            </option>
          ))}
        </select>
        <SelectChevron />
      </div>

      <div className="relative">
        <select
          aria-label="Font size"
          data-testid="ribbon-font-size"
          value={fontHeight != null && (FONT_SIZES as readonly number[]).includes(fontHeight) ? String(fontHeight) : ''}
          onChange={(e) => onFontHeight(Number(e.target.value))}
          className={`${selectClass} w-[58px] pr-5`}
        >
          {(fontHeight == null || !(FONT_SIZES as readonly number[]).includes(fontHeight)) && (
            <option value="" disabled>
              {fontHeight ?? ''}
            </option>
          )}
          {FONT_SIZES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
      </div>
    </div>
  )
}

/** Placed right after B/I/U/S, matching the mockup's own described order:
 *  "... B/I/U/S, text colour, highlight, lists, ..." — and reused, unchanged,
 *  as the floating selection toolbar's color icon (CRITIQUE.md finding #7,
 *  same root cause as #4: there was nothing to wire it to until this file
 *  existed). */
export function ColorPickers({ textColor, highlightColor, onTextColor, onHighlightColor }: ColorPickersProps) {
  const textColorInputRef = useRef<HTMLInputElement>(null)
  const highlightColorInputRef = useRef<HTMLInputElement>(null)

  const textColorHex = unoColorToHex(textColor, '#1a1a1a')
  const highlightColorHex = unoColorToHex(highlightColor, '#ffe066')

  return (
    <div className="flex shrink-0 items-center gap-0.5" data-testid="office-color-pickers">
      <div className="relative grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md text-ink hover:bg-paper-3">
        <button
          type="button"
          aria-label="Text color"
          data-testid="ribbon-text-color"
          title="Text color"
          onClick={() => textColorInputRef.current?.click()}
          className="grid h-full w-full place-items-center"
        >
          <IconTextColor size={16} barColor={textColorHex} />
        </button>
        <input
          ref={textColorInputRef}
          type="color"
          aria-hidden="true"
          tabIndex={-1}
          data-testid="ribbon-text-color-input"
          value={textColorHex}
          onChange={(e) => onTextColor(hexToUnoColor(e.target.value))}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        />
      </div>

      <div className="relative grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md text-ink hover:bg-paper-3">
        <button
          type="button"
          aria-label="Highlight color"
          data-testid="ribbon-highlight-color"
          title="Highlight color"
          onClick={() => highlightColorInputRef.current?.click()}
          className="grid h-full w-full place-items-center"
        >
          <IconHighlighter size={16} barColor={highlightColorHex} />
        </button>
        <input
          ref={highlightColorInputRef}
          type="color"
          aria-hidden="true"
          tabIndex={-1}
          data-testid="ribbon-highlight-color-input"
          value={highlightColorHex}
          onChange={(e) => onHighlightColor(hexToUnoColor(e.target.value))}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        />
      </div>
    </div>
  )
}
