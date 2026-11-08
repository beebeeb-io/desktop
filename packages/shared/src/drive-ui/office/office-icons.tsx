/**
 * Office-editor-only icons (task 1567) not in `@beebeeb/shared`'s icon set
 * (undo/redo, alignment, lists, indent, chevron, minus, collapse). Paths
 * adapted from the approved mockup (design/office-editor.html's `<symbol>`
 * defs) so the ribbon is pixel-consistent with what Guus signed off on.
 * Monochrome, `currentColor`-only, per brand rules — no icon library added
 * for a dozen glyphs.
 *
 * CRITIQUE.md finding #11 (task 1567, low): the base stroke weight here was
 * 1.5 vs `@beebeeb/shared`'s icon set's 1.6, a sub-pixel drift risk when
 * mixed in the same ribbon row. Fixed to 1.6 to match. A few icons
 * deliberately use a lighter 1.2-1.4 for visual balance at their specific
 * shape (e.g. the dense grid/table glyphs) — untouched, that's normal
 * per-icon weight tuning, not the reported drift.
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

export function IconUndo(props: IconProps) {
  return svg(
    <path
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      d="M5.5 4 2.5 7l3 3M2.8 7h6.7a3.5 3.5 0 0 1 0 7H7"
    />,
    props,
  )
}

export function IconRedo(props: IconProps) {
  return svg(
    <path
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      d="M10.5 4l3 3-3 3M13.2 7H6.5a3.5 3.5 0 0 0 0 7H9"
    />,
    props,
  )
}

export function IconAlignLeft(props: IconProps) {
  return svg(<path d="M2 4h12M2 8h8M2 12h10" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />, props)
}
export function IconAlignCenter(props: IconProps) {
  return svg(<path d="M2 4h12M4 8h8M3 12h10" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />, props)
}
export function IconAlignRight(props: IconProps) {
  return svg(<path d="M2 4h12M6 8h8M4 12h10" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />, props)
}
export function IconAlignJustify(props: IconProps) {
  return svg(<path d="M2 4h12M2 8h12M2 12h9" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />, props)
}

export function IconListBullet(props: IconProps) {
  return svg(
    <>
      <circle cx="3" cy="4" r="1" fill="currentColor" />
      <circle cx="3" cy="8" r="1" fill="currentColor" />
      <circle cx="3" cy="12" r="1" fill="currentColor" />
      <path d="M6.4 4h7.6M6.4 8h7.6M6.4 12h7.6" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" />
    </>,
    props,
  )
}
export function IconListNumbered(props: IconProps) {
  return svg(
    <>
      <rect x="2" y="3" width="2" height="2" rx=".4" fill="currentColor" />
      <rect x="2" y="7" width="2" height="2" rx=".4" fill="currentColor" />
      <rect x="2" y="11" width="2" height="2" rx=".4" fill="currentColor" />
      <path d="M6.4 4h7.6M6.4 8h7.6M6.4 12h7.6" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" />
    </>,
    props,
  )
}

export function IconIndentMore(props: IconProps) {
  return svg(
    <>
      <path d="M2 3.5h12M2 12.5h12M2 8h5" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" />
      <path
        d="M9.2 5.3 12.2 8l-3 2.7"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </>,
    props,
  )
}
export function IconIndentLess(props: IconProps) {
  return svg(
    <>
      <path d="M2 3.5h12M2 12.5h12M6.8 8h7.2" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" />
      <path
        d="M6.8 5.3 3.8 8l3 2.7"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </>,
    props,
  )
}

export function IconChevronDown(props: IconProps) {
  return svg(
    <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" />,
    props,
  )
}

export function IconMinus(props: IconProps) {
  return svg(<path d="M3 8h10" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />, props)
}

export function IconCollapse(props: IconProps) {
  return svg(
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1.4" fill="none" stroke="currentColor" strokeWidth={1.3} />
      <path d="M6.5 2.5v11" stroke="currentColor" strokeWidth={1.3} />
    </>,
    props,
  )
}

export function IconTable(props: IconProps) {
  return svg(
    <>
      <rect x="2" y="3" width="12" height="10" rx="1" fill="none" stroke="currentColor" strokeWidth={1.4} />
      <path d="M2 7h12M2 10.3h12M6.3 3v10M10.6 3v10" stroke="currentColor" strokeWidth={1.2} />
    </>,
    props,
  )
}

/** CRITIQUE.md findings #4/#7 (task 1567): a bare "A" glyph with a colored
 *  bar underneath reflecting the currently-applied `.uno:Color` — same
 *  iconography Word/Docs use for their font-color button. `barColor`
 *  defaults to the brand accent only as a placeholder before any real state
 *  has loaded; the ribbon/toolbar always pass the live value. */
export function IconTextColor({ barColor = 'currentColor', ...props }: IconProps & { barColor?: string }) {
  return svg(
    <>
      <path
        d="M5.2 10.5 8 3.6l2.8 6.9M6 8.4h4"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <rect x="2.5" y="12.3" width="11" height="2" rx="0.6" fill={barColor} />
    </>,
    props,
  )
}

/** CRITIQUE.md findings #4/#7: a highlighter/marker glyph with a colored bar
 *  reflecting the currently-applied `.uno:CharBackColor`. */
export function IconHighlighter({ barColor = 'currentColor', ...props }: IconProps & { barColor?: string }) {
  return svg(
    <>
      <path
        d="M9.8 2.6 13 5.8 7.7 11.1 4 11.5l.4-3.7z"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.3}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M4.4 8.2 7.4 11.2" stroke="currentColor" strokeWidth={1.3} strokeLinecap="round" />
      <rect x="2.5" y="12.3" width="11" height="2" rx="0.6" fill={barColor} />
    </>,
    props,
  )
}
