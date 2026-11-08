/**
 * OutlinePane (task 1567, Writer only) — reads `bbOffice.getOutline()`,
 * clicking an entry calls `bbOffice.goToHeading(index)`. Reuses the visual
 * language of design/office-editor.html's outline pane (the mockup notes it
 * as "reused from editor-1563" in spirit — a left rail list, not a new
 * pattern).
 */

import { useEffect, useState } from 'react'
import type { OutlineHeading } from '../lib/office/bb-office-bridge'
import { IconCollapse } from './office-icons'

export interface OutlinePaneProps {
  headings: OutlineHeading[]
  onSelect: (index: number) => void
  /** CRITIQUE.md finding #5 (task 1567): while the engine is still booting,
   *  the approved "firstload" mockup screen shows this pane already in
   *  place with 3 grey skeleton bars instead of real headings (or "No
   *  headings yet") — not absent, which is what shipped before this fix
   *  (office-editor.tsx didn't render `<OutlinePane>` at all until
   *  `docReady`). */
  loading?: boolean
}

/**
 * Task 1585 item 3: below this width (phone portrait) the 216 px pane took
 * about half the screen away from the document. There it starts collapsed to
 * its 32 px rail, and when opened it floats OVER the canvas instead of
 * squeezing it (so the engine canvas is not resized every time the outline is
 * toggled), and closes again once a heading is picked. Tailwind's `sm`.
 */
export const OUTLINE_NARROW_QUERY = '(max-width: 639px)'

function isNarrowViewport(): boolean {
  try {
    return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia(OUTLINE_NARROW_QUERY).matches
  } catch {
    return false
  }
}

export function OutlinePane({ headings, onSelect, loading = false }: OutlinePaneProps) {
  const [narrow, setNarrow] = useState(isNarrowViewport)
  const [collapsed, setCollapsed] = useState(isNarrowViewport)
  const [activeIndex, setActiveIndex] = useState<number | null>(null)

  // Crossing the breakpoint (rotating a phone, resizing a window) resets the
  // pane to that width's default; a user's own toggle holds until then.
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    const mql = window.matchMedia(OUTLINE_NARROW_QUERY)
    const onChange = (e: MediaQueryListEvent) => {
      setNarrow(e.matches)
      setCollapsed(e.matches)
    }
    mql.addEventListener('change', onChange)
    return () => mql.removeEventListener('change', onChange)
  }, [])

  if (collapsed) {
    return (
      <div className="flex w-8 shrink-0 flex-col items-center border-r border-line bg-paper-2 pt-3" data-testid="office-outline-rail">
        <button
          type="button"
          aria-label="Show outline"
          aria-expanded={false}
          data-testid="office-outline-show"
          onClick={() => setCollapsed(false)}
          className="grid h-6 w-6 place-items-center rounded-md text-ink-3 hover:bg-paper-3"
        >
          <IconCollapse size={13} />
        </button>
      </div>
    )
  }

  const pane = (
    // IMPORTANT: `data-testid="office-outline-pane"` is an existing e2e
    // ready-signal (e2e/1567-office-editor.spec.ts waits on it as its proxy
    // for "the document is actually open" before dispatching into the
    // canvas) — it must stay reserved for the REAL, non-loading render.
    // Giving the skeleton its own `office-outline-pane-loading` id (found by
    // actually running the spec: an earlier version of this fix put the
    // real testid on this wrapper unconditionally, and the spec's dispatch
    // then raced a still-null document model — a real regression, not
    // hypothetical).
    <div
      className={
        narrow
          ? // Floats over the canvas (see OUTLINE_NARROW_QUERY). Absolute
            // within the editor's body row, which is `relative`.
            'absolute inset-y-0 left-0 z-20 flex w-[min(260px,82vw)] flex-col border-r border-line bg-paper-2 shadow-2'
          : 'flex w-[216px] shrink-0 flex-col border-r border-line bg-paper-2'
      }
      data-testid={loading ? 'office-outline-pane-loading' : 'office-outline-pane'}
      data-overlay={narrow ? 'true' : undefined}
    >
      <div className="flex items-center justify-between px-3.5 pb-2 pt-3.5">
        <span className="text-[10.5px] font-semibold uppercase tracking-wider text-ink-3">Outline</span>
        <button
          type="button"
          aria-label="Collapse outline"
          aria-expanded={true}
          data-testid="office-outline-collapse"
          onClick={() => setCollapsed(true)}
          className="grid h-[22px] w-[22px] place-items-center rounded-md text-ink-3 hover:bg-paper-3"
        >
          <IconCollapse size={13} />
        </button>
      </div>
      <div className="flex flex-col gap-px overflow-auto px-2 pb-3.5" data-testid={loading ? 'office-outline-skeleton' : undefined}>
        {loading ? (
          <div className="flex flex-col gap-2.5 px-2.5 pt-1">
            <div className="h-2 w-[85%] rounded-full bg-line" />
            <div className="h-2 w-[55%] rounded-full bg-line" />
            <div className="h-2 w-[65%] rounded-full bg-line" />
          </div>
        ) : (
          <>
            {headings.length === 0 && (
              <div className="px-2.5 py-1.5 text-[12px] text-ink-4">No headings yet</div>
            )}
          </>
        )}
        {!loading && headings.map((h, i) => (
          <button
            key={`${i}-${h.text}`}
            type="button"
            data-testid={`outline-item-${i}`}
            onClick={() => {
              setActiveIndex(i)
              onSelect(i)
              if (narrow) setCollapsed(true)
            }}
            className={`rounded-md px-2.5 py-1.5 text-left text-[12.5px] ${
              h.level >= 2 ? 'pl-[22px] text-[12px]' : ''
            } ${activeIndex === i ? 'bg-paper-3 font-medium text-ink' : 'text-ink-3 hover:bg-paper-3'}`}
          >
            {h.text}
          </button>
        ))}
      </div>
    </div>
  )
  if (!narrow) return pane
  // Keep the 32 px rail's footprint under the floating pane so opening it
  // never reflows (and so never resizes) the engine canvas.
  return (
    <>
      <div aria-hidden="true" className="w-8 shrink-0 border-r border-line bg-paper-2" />
      {pane}
    </>
  )
}
