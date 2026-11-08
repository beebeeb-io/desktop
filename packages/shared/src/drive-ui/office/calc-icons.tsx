/**
 * Calc-only icons (task 1567, Calc lane) not already in office-icons.tsx
 * (shared -- Undo/Redo/Align/Table are reused from there read-only) or
 * `@beebeeb/shared`. Kept in their own file, mirroring office-icons.tsx's own
 * `svg()` helper locally rather than importing it, so this lane never needs
 * to touch a file other lanes are also extending. Monochrome,
 * `currentColor`-only, per brand rules.
 */

interface IconProps {
  size?: number
  className?: string
}

function svg(children: React.ReactNode, { size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" className={className} aria-hidden="true">
      {children}
    </svg>
  )
}

/** Sort (matching the mockup's `#i-sort` glyph: two arrows, ascending). */
export function IconSort(props: IconProps) {
  return svg(
    <path
      d="M4 3v10M4 13 1.5 10.5M4 13l2.5-2.5M12 13V3M12 3l-2.5 2.5M12 3l2.5 2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
    />,
    props,
  )
}

/** Funnel (AutoFilter). */
export function IconFilter(props: IconProps) {
  return svg(
    <path d="M2.5 3h11L9 8.2v4.3L7 14V8.2L2.5 3Z" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinejoin="round" strokeLinecap="round" />,
    props,
  )
}

/** Small "+" for the sheet-tab bar's add button -- deliberately its own tiny
 *  glyph rather than importing `@beebeeb/shared`'s "plus" Icon, so this file
 *  has zero cross-package dependency for one shape. */
export function IconPlusSmall(props: IconProps) {
  return svg(<path d="M8 3v10M3 8h10" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" />, props)
}

/** Decorative fill-color swatch (the mockup's `.rswatch`) -- no color picker
 *  wired up yet (flagged, not silently dropped, in calc-commands.ts's own
 *  header comment), so this is a static amber underline glyph, matching the
 *  Writer ribbon's equally-decorative treatment of the same control. */
export function IconFillSwatch(props: IconProps) {
  return svg(
    <>
      <text x="3" y="10.5" fontSize="8" fontWeight={700} fill="currentColor">
        A
      </text>
      <rect x="2.5" y="13" width="8" height="1.6" rx="0.6" fill="currentColor" />
    </>,
    props,
  )
}
