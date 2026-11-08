// The Office canvases: the outline and page (Docs), the cell box, grid and sheet tabs (Sheets), the filmstrip, slide
// and present mode (Slides). In the app the engine draws the document itself (EngineFrame); the frames draw the
// board's documents with these same pieces around them. Selection is neutral grey; no amber.
import { useEffect, useState, type ReactNode, type RefObject } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FOCUS } from '../../files/ui'
import type { DocFixture, SlideFixture } from '../../../source/fixtures/office'
import { Glyph } from './Ribbon'

const PAGE_BG = 'bg-paper-3'
/** Document ink: the page is white paper in both themes. */
const PAPER = { background: 'oklch(1 0 0)', color: 'oklch(0.22 0.01 70)' }

/** Outline: the document's headings; a click goes there. */
export function OutlinePane({ items, current, onGo, onCollapse }: { items: [string, number][]; current?: number; onGo?: (i: number) => void; onCollapse?: () => void }) {
  return (
    <nav aria-label="Outline" data-outline className="flex w-[210px] flex-none flex-col gap-0.5 overflow-hidden border-r border-line bg-paper-2 px-2 py-3">
      <div className="flex items-center px-2 pb-1">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-ink-3">Outline</span><span className="flex-1" />
        <button type="button" aria-label="Collapse outline" onClick={onCollapse} className={`grid h-6 w-6 place-items-center rounded text-ink-3 hover:bg-paper-3 ${FOCUS}`}><span className="rotate-180"><Icon name="chevron-right" size={12} /></span></button>
      </div>
      {items.length === 0 && <p className="m-0 px-2 text-[12px] text-ink-3">No headings yet</p>}
      {items.map(([t, level], i) => (
        <a key={`${t}-${i}`} href={`#heading-${i}`} data-heading aria-current={i === current ? 'location' : undefined} onClick={(e) => { e.preventDefault(); onGo?.(i) }}
          className={`truncate rounded-md py-1 text-[12.5px] no-underline ${i === current ? 'bg-paper-3 text-ink' : 'text-ink-2 hover:bg-paper-3'} ${level === 0 ? 'font-semibold' : ''}`}
          style={{ paddingLeft: 8 + level * 12 }}>{t}</a>
      ))}
    </nav>
  )
}

/** A Docs page drawn from a fixture, with the selection and the floating toolbar where the board puts them. */
export function DocPage({ doc, selection, table, children }: { doc: DocFixture; selection?: boolean; table?: string[][]; children?: ReactNode }) {
  return (
    <div className={`relative flex min-w-0 flex-1 justify-center overflow-hidden pt-6 ${PAGE_BG}`}>
      <article data-page className="flex h-[760px] w-[520px] flex-none flex-col gap-2 rounded-sm px-12 py-11 shadow-md" style={PAPER}>
        <h2 className="m-0 text-[20px] font-bold leading-tight">{doc.title}</h2>
        <p className="m-0 text-[12px] opacity-60">{doc.sub}</p>
        {doc.sections.map((s, i) => (
          <section key={s.heading}>
            <h3 className="m-0 mt-2 text-[14px] font-semibold">{s.heading}</h3>
            <p className="m-0 text-[12.5px] leading-5">{s.text}{s.selected && <span data-selection={selection || undefined} className={selection ? 'rounded-sm bg-[oklch(0.22_0.01_70/0.14)]' : ''}>{s.selected}</span>}</p>
            {table && i === 1 && (
              <table data-inserted-table className="mt-2 w-full border-collapse text-[11px]">
                <tbody>{table.map((row, r) => <tr key={r}>{row.map((c, j) => <td key={j} className={`border border-[oklch(0.22_0.01_70/0.3)] px-2 py-1 ${r === 0 ? 'font-semibold' : ''}`}>{c}</td>)}</tr>)}</tbody>
              </table>
            )}
          </section>
        ))}
      </article>
      {children}
    </div>
  )
}

/** The cell box, fx and the formula (DW-1508, DW-1559). Enter goes to the cell or range typed. */
export function FormulaBar({ cell, formula, focused, message, onGo }: { cell: string; formula: string; focused?: boolean; message?: string; onGo?: (ref: string) => boolean }) {
  const [value, setValue] = useState(cell)
  const [note, setNote] = useState<string | undefined>(message)
  useEffect(() => { setValue(cell) }, [cell])
  const go = () => {
    const ok = isCellRef(value) && (onGo?.(value.trim().toUpperCase()) ?? true)
    setNote(ok ? `Went to ${value.trim().toUpperCase()}` : 'Not a valid cell or range')
  }
  return (
    <div data-formula-bar className="flex h-[34px] flex-none items-center gap-2 border-b border-line bg-paper px-3">
      <input aria-label="Cell or range" value={value} ref={focused ? (el) => el?.focus() : undefined} onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === 'F2') { e.preventDefault(); go() } }}
        title="Type a cell or range and press Enter to go there"
        className={`h-6 w-[92px] rounded-md border bg-paper px-2 text-center font-mono text-[12px] text-ink outline-none ${focused ? 'border-ink-2' : 'border-line'} focus:border-ink-2`} />
      <span className="text-ink-3"><Glyph name="fx" /></span>
      <span className="flex-1 truncate font-mono text-[12px] text-ink">{formula}</span>
      {(note ?? (focused ? 'Type a cell or range and press Enter to go there' : undefined)) && <span role="status" className="text-[12px] text-ink-3">{note ?? 'Type a cell or range and press Enter to go there'}</span>}
    </div>
  )
}

export function isCellRef(v: string): boolean {
  return /^\s*\$?[A-Za-z]{1,3}\$?\d{1,7}(:\$?[A-Za-z]{1,3}\$?\d{1,7})?\s*$/.test(v)
}

/** The grid: column letters, row numbers, text left, numbers right in tabular figures; the selection neutral. */
export function SheetGrid({ cols, rows, selection, current }: { cols: string[]; rows: string[][]; selection?: { col: number; from: number; to: number }; current?: { col: number; row: number } }) {
  return (
    <div className="min-h-0 flex-1 overflow-hidden bg-paper">
      <table data-grid className="border-collapse text-[12px] text-ink">
        <thead><tr><th className="h-6 w-10 bg-paper-2 shadow-[inset_-0.5px_-0.5px_0_var(--color-line)]" />{cols.map((c) => <th key={c} scope="col" className="h-6 w-[118px] bg-paper-2 font-medium text-ink-3 shadow-[inset_-0.5px_-0.5px_0_var(--color-line)]">{c}</th>)}</tr></thead>
        <tbody>
          {Array.from({ length: 19 }, (_, r) => (
            <tr key={r} data-row={r + 1}>
              <th scope="row" className="h-6 bg-paper-2 text-center font-mono text-[11px] font-normal text-ink-3 shadow-[inset_-0.5px_-0.5px_0_var(--color-line)]">{r + 1}</th>
              {cols.map((c, ci) => {
                const v = rows[r]?.[ci] ?? ''
                const inSel = selection && ci === selection.col && r >= selection.from && r <= selection.to
                const cur = current && ci === current.col && r === current.row
                const num = ci > 0 && r > 0
                return (
                  <td key={c} aria-selected={inSel || cur || undefined} className={`px-2 ${num ? 'text-right font-mono text-[11.5px] tabular-nums' : ''} ${inSel ? 'bg-paper-3' : ''} ${r === 0 || rows[r]?.[0] === 'Total' ? 'font-semibold' : ''}`}
                    style={{ boxShadow: cur ? 'inset 0 0 0 2px var(--color-ink)' : 'inset -0.5px -0.5px 0 var(--color-line)' }}>{v}</td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** Sheet tabs; double-click renames; + is dimmed in this editor. */
export function SheetTabs({ tabs, active, onPick, onRename }: { tabs: string[]; active: number; onPick?: (i: number) => void; onRename?: (i: number, name: string) => void }) {
  const [editing, setEditing] = useState<number | null>(null)
  return (
    <div role="tablist" aria-label="Sheets" data-sheet-tabs className="flex h-[30px] flex-none items-center gap-1 border-t border-line bg-paper-2 px-2">
      {tabs.map((t, i) => editing === i ? (
        <input key={t} aria-label="Sheet name" defaultValue={t} ref={(el) => el?.select()} onBlur={(e) => { setEditing(null); onRename?.(i, e.target.value) }} onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); if (e.key === 'Escape') setEditing(null) }} className="h-[22px] w-20 rounded border border-ink-3 bg-paper px-2 text-[12px] text-ink outline-none" />
      ) : (
        <button key={t} type="button" role="tab" aria-selected={i === active} onClick={() => onPick?.(i)} onDoubleClick={() => setEditing(i)}
          className={`grid h-[22px] place-items-center rounded-[5px] px-3 text-[12px] ${i === active ? 'bg-paper font-semibold text-ink shadow-[0_0_0_0.5px_var(--color-line)]' : 'text-ink-2 hover:bg-paper-3'} ${FOCUS}`}>{t}</button>
      ))}
      <button type="button" aria-label="Add sheet" disabled title="Adding sheets isn't available in this editor yet" className="grid h-[22px] w-[22px] place-items-center rounded text-ink-3 opacity-50"><Icon name="plus" size={13} /></button>
    </div>
  )
}

function SlideBody({ slide, scale }: { slide: SlideFixture; scale: number }) {
  return (
    <>
      <div className="font-bold leading-tight tracking-tight" style={{ fontSize: 26 * scale }}>{slide.title}</div>
      {slide.sub && <div style={{ fontSize: 13 * scale, opacity: 0.65 }}>{slide.sub}</div>}
      {slide.points.length > 0 && <ul className="m-0 list-disc" style={{ paddingLeft: '1.1em', fontSize: 13.5 * scale, lineHeight: 1.5 }}>{slide.points.map((p) => <li key={p}>{p}</li>)}</ul>}
      {slide.photo && <span aria-label="Photo" role="img" className="min-h-[14px] flex-1 rounded bg-[linear-gradient(135deg,oklch(0.78_0.04_230),oklch(0.62_0.05_200)_55%,oklch(0.82_0.06_80))]" />}
    </>
  )
}

/** The filmstrip: the slides themselves, small; right-click offers the slide actions. */
export function Filmstrip({ slides, current, onPick, onMenu }: { slides: SlideFixture[]; current: number; onPick?: (i: number) => void; onMenu?: (i: number, x: number, y: number) => void }) {
  return (
    <nav aria-label="Slides" data-filmstrip className="flex w-[190px] flex-none flex-col gap-2.5 overflow-hidden border-r border-line bg-paper-2 p-3">
      <span className="px-1 text-[11px] font-semibold uppercase tracking-wide text-ink-3">Slides</span>
      {slides.map((s, i) => (
        <button key={s.title} type="button" data-slide-thumb aria-current={i === current ? 'true' : undefined} aria-label={`Slide ${i + 1}: ${s.title}`} onClick={() => onPick?.(i)} onContextMenu={(e) => { e.preventDefault(); onMenu?.(i, e.clientX, e.clientY) }} className={`flex items-start gap-2 text-left ${FOCUS}`}>
          <span className="w-3 pt-1 font-mono text-[11px] text-ink-3">{i + 1}</span>
          <span className="flex h-[79px] w-[140px] flex-col gap-0.5 overflow-hidden rounded p-2" style={{ ...PAPER, boxShadow: i === current ? '0 0 0 2px var(--color-ink)' : '0 1px 4px oklch(0 0 0 / 0.14)' }}>
            <SlideBody slide={s} scale={0.33} />
          </span>
        </button>
      ))}
    </nav>
  )
}

export function SlideCanvas({ slide }: { slide: SlideFixture }) {
  return (
    <div className={`grid min-w-0 flex-1 place-items-center ${PAGE_BG}`}>
      <div data-slide className="flex h-[360px] w-[640px] flex-none flex-col gap-3 rounded p-10 shadow-lg" style={PAPER}><SlideBody slide={slide} scale={1} /></div>
    </div>
  )
}

/** Present (DW-1561): the slide fills the screen on black; arrows, Space or a click move; Esc returns. */
export function PresentView({ slides, index, onIndex, onExit }: { slides: SlideFixture[]; index: number; onIndex: (i: number) => void; onExit: () => void }) {
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onExit()
      else if (e.key === 'ArrowRight' || e.key === ' ') onIndex(Math.min(slides.length - 1, index + 1))
      else if (e.key === 'ArrowLeft') onIndex(Math.max(0, index - 1))
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [index, slides.length, onIndex, onExit])
  return (
    <div role="dialog" aria-label="Present" data-present className="fixed inset-0 z-50 grid place-items-center bg-black">
      <div className="flex aspect-video w-[min(100vw,177.78vh)] flex-col gap-5 p-16" style={PAPER}><SlideBody slide={slides[index]} scale={1.7} /></div>
      <button type="button" aria-label="Next slide" onClick={() => onIndex(Math.min(slides.length - 1, index + 1))} className="absolute inset-0 cursor-default bg-transparent" />
      <button type="button" onClick={onExit} className="absolute right-4 top-3 z-10 font-mono text-[12px] text-white/70">esc</button>
      <span className="absolute bottom-3 right-4 font-mono text-[12px] text-white/70">{index + 1} / {slides.length}</span>
    </div>
  )
}

/** The engine's own document view, from the cached bundle on the `bboffice` scheme. */
export function EngineFrame({ src, frameRef, onLoad }: { src: string; frameRef: RefObject<HTMLIFrameElement | null>; onLoad: () => void }) {
  return (
    <div className={`relative min-w-0 flex-1 overflow-hidden ${PAGE_BG}`}>
      <iframe ref={frameRef} src={src} onLoad={onLoad} title="Document" sandbox="allow-scripts allow-same-origin" className="absolute inset-0 h-full w-full border-0" />
    </div>
  )
}
