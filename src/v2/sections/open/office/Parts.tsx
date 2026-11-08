// The Office editor's parts around the document (DW-1551..1558, DW-1511): the status bar, the floating selection
// toolbar, the command palette, the conflict dialog, About, the preparing and failed states, the older-format note
// and the New document dialog. The board's words; Amber only on encryption state and the one primary action.
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { TEMPLATE_DEFS, type OfficeApp, type TemplateDef } from '@beebeeb/shared/drive-ui'
import { BTN, BTN_AMBER, FOCUS, Modal } from '../../files/ui'
import { Glyph } from './Ribbon'

const dot = <span aria-hidden="true">·</span>

/** The editor's status bar, in the window's status-line place. */
export function OfficeStatus({ left, dirty, conflict, version, time, loading, zoom = 100, onZoom, onAbout }: {
  left: ReactNode[]; dirty?: boolean; conflict?: boolean; version: number; time: string; loading?: boolean; zoom?: number; onZoom?: (z: number) => void; onAbout?: () => void
}) {
  const right = loading ? 'read-only for now' : conflict ? 'not saved · conflict' : dirty ? 'unsaved changes' : <>saved as version {version} {'·'} <span className="font-mono">{time}</span></>
  return (
    <footer role="status" data-office-status className="flex h-7 flex-none items-center gap-1.5 border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
      {loading
        ? <><span className="h-1.5 w-1.5 rounded-full bg-ink-3" /><b className="font-medium text-ink-2">Preparing the editor on this PC…</b></>
        : left.map((l, i) => <span key={i} className="flex items-center gap-1.5">{i > 0 && dot}<span>{l}</span></span>)}
      <span className="flex-1" />
      <span className="inline-flex items-center gap-1 font-semibold text-amber-deep"><Icon name="lock" size={11} />Encrypted</span>
      {dot}<span data-save-state>{right}</span>
      <span aria-hidden="true" className="mx-1 h-4 w-px bg-line" />
      <span className="inline-flex items-center gap-1">
        <button type="button" aria-label="Zoom out" onClick={() => onZoom?.(Math.max(50, zoom - 10))} className={`grid h-5 w-5 place-items-center rounded hover:bg-paper-3 ${FOCUS}`}>−</button>
        <span className="font-mono">{zoom}%</span>
        <button type="button" aria-label="Zoom in" onClick={() => onZoom?.(Math.min(200, zoom + 10))} className={`grid h-5 w-5 place-items-center rounded hover:bg-paper-3 ${FOCUS}`}><Icon name="plus" size={12} /></button>
      </span>
      <span aria-hidden="true" className="mx-1 h-4 w-px bg-line" />
      <a href="#about" onClick={(e) => { e.preventDefault(); onAbout?.() }} className="text-ink-2 underline underline-offset-2">About</a>
    </footer>
  )
}

/** Bold, Italic, Underline, Text colour, Insert link over a selection (DW-1507). */
export function FloatingToolbar({ x, y, onRun }: { x: number | string; y: number; onRun?: (id: string) => void }) {
  const b = (id: string, label: string, body: ReactNode) => (
    <button key={id} type="button" aria-label={label} title={label} onClick={() => onRun?.(id)} className={`grid h-7 w-7 place-items-center rounded-md text-ink hover:bg-paper-3 ${FOCUS}`}>{body}</button>
  )
  return (
    <div role="toolbar" aria-label="Selection" data-floating-toolbar className="absolute z-10 flex h-[34px] items-center gap-0.5 rounded-[10px] border border-line bg-paper px-1 shadow-lg" style={{ left: x, top: y }}>
      {b('bold', 'Bold', <b className="text-[13.5px] font-extrabold">B</b>)}
      {b('italic', 'Italic', <b className="font-serif text-[13.5px] italic">I</b>)}
      {b('underline', 'Underline', <b className="text-[13.5px] underline">U</b>)}
      {b('text-colour', 'Text colour', <Glyph name="color" />)}
      {b('insert-link', 'Insert link', <Icon name="link" size={15} />)}
    </div>
  )
}

/** `pinned`: the recent file the list always offers (Files), whatever is typed. */
export interface PaletteRow { id: string; label: string; group: 'Styles' | 'Format' | 'Insert' | 'Document' | 'Files'; shortcut?: string; pinned?: boolean }

/** Every command, style and file by typing (DW-1551). Searches only what is on this PC. */
export function CommandPalette({ rows, initialQuery = '', onRun, onClose }: { rows: PaletteRow[]; initialQuery?: string; onRun: (row: PaletteRow) => void; onClose: () => void }) {
  const [query, setQuery] = useState(initialQuery)
  const [active, setActive] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const hits = useMemo(() => {
    const q = query.trim().toLowerCase()
    return q ? rows.filter((r) => r.pinned || r.label.toLowerCase().includes(q)) : rows
  }, [rows, query])
  const groups = useMemo(() => {
    const by = new Map<string, PaletteRow[]>()
    for (const r of hits) by.set(r.group, [...(by.get(r.group) ?? []), r])
    return [...by.entries()]
  }, [hits])
  useEffect(() => { setTimeout(() => input.current?.focus(), 0) }, [])
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setActive((a) => Math.min(hits.length - 1, a + 1)) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setActive((a) => Math.max(0, a - 1)) }
    else if (e.key === 'Enter' && hits[active]) { e.preventDefault(); onRun(hits[active]) }
    else if (e.key === 'Escape') { e.preventDefault(); onClose() }
  }
  let n = -1
  return (
    <div role="presentation" className="fixed inset-0 z-50 flex items-start justify-center bg-ink/25 pt-24" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div role="dialog" aria-modal="true" aria-label="Commands" data-palette className="w-[560px] max-w-[calc(100vw-48px)] overflow-hidden rounded-xl border border-line bg-paper shadow-2xl">
        <div className="flex h-12 items-center gap-2 border-b border-line px-4">
          <Icon name="search" size={15} />
          <input ref={input} value={query} onKeyDown={onKey} aria-activedescendant={hits[active] ? `palette-${hits[active].id}` : undefined} onChange={(e) => { setQuery(e.target.value); setActive(0) }} aria-label="Search commands, styles, files" placeholder="Search commands, styles, files…" className="h-full flex-1 bg-transparent text-[14px] text-ink outline-none placeholder:text-ink-3" />
        </div>
        <div role="listbox" aria-label="Results" className="max-h-[360px] overflow-y-auto py-1">
          {groups.length === 0 && <p className="m-0 px-4 py-3 text-[13px] text-ink-3">Nothing matches “{query}”.</p>}
          {groups.map(([g, list]) => (
            <div key={g} role="group" aria-label={g}>
              <div className="px-4 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-ink-3">{g}</div>
              {list.map((r) => {
                n += 1
                const i = n
                return (
                  <button type="button" key={r.id} id={`palette-${r.id}`} role="option" tabIndex={-1} aria-selected={i === active} data-palette-row onMouseEnter={() => setActive(i)} onClick={() => onRun(r)}
                    className={`flex h-8 w-full cursor-default items-center gap-3 px-4 text-left text-[13px] text-ink ${i === active ? 'bg-paper-3' : ''}`}>
                    <span className="flex-1 truncate">{r.label}</span>
                    {r.shortcut && <span className="font-mono text-[11.5px] text-ink-3">{r.shortcut}</span>}
                  </button>
                )
              })}
            </div>
          ))}
        </div>
        <div className="flex h-9 items-center gap-2 border-t border-line px-4 text-[12px] text-ink-3">
          <Icon name="lock" size={11} /><span className="flex-1">Runs entirely on this device</span><kbd className="font-mono text-[11px]">esc</kbd>
        </div>
      </div>
    </div>
  )
}

export type ConflictChoice = 'save-as-new-version' | 'keep-both' | 'discard'

/** Someone else saved first (DW-1554): never overwrites silently. */
export function ConflictDialog({ latest, opened, saving, onChoose, onCancel }: { latest: number; opened: number; saving?: boolean; onChoose: (c: ConflictChoice) => void; onCancel: () => void }) {
  return (
    <Modal title="Someone else saved first" label="Someone else saved first" onClose={onCancel} width={500}
      footer={<div className="flex w-full flex-col items-stretch gap-2">
        <button type="button" disabled={saving} onClick={() => onChoose('save-as-new-version')} className={`${BTN_AMBER} disabled:opacity-50`}>Save as a new version anyway</button>
        <button type="button" disabled={saving} onClick={() => onChoose('keep-both')} className={BTN}>Keep both — save mine as a separate file</button>
        <button type="button" disabled={saving} onClick={() => onChoose('discard')} className={BTN}>Discard my changes and reopen version {latest}</button>
        <button type="button" onClick={onCancel} className={BTN}>Cancel</button>
      </div>}>
      <p className="m-0">This file was saved as version {latest} while you were editing (you opened version {opened}). Office documents can’t be diffed line by line — choose how to keep your edit.</p>
    </Modal>
  )
}

export const OFFICE_SOURCE_URL = 'https://github.com/beebeeb-io/office'

/** The editor's licence notice (DW-1557), as the open-source licences require. */
export function AboutPanel({ onClose, onOpenLink, noticesAvailable }: { onClose: () => void; onOpenLink?: (which: 'source' | 'licenses') => void; noticesAvailable: boolean }) {
  return (
    <Modal title="About this editor" label="About this editor" onClose={onClose} width={480} footer={<span className="flex w-full justify-end"><button type="button" onClick={onClose} className={BTN}>Close</button></span>}>
      <p className="m-0">This editor is LibreOffice, compiled to run entirely on this device. Beebeeb’s changes and the build recipe are open source.</p>
      <dl className="m-0 mt-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-[13px]">
        <dt className="text-ink-3">Source</dt>
        <dd className="m-0"><a href={OFFICE_SOURCE_URL} onClick={(e) => { e.preventDefault(); onOpenLink?.('source') }} className="font-mono text-[12.5px] text-ink underline">github.com/beebeeb-io/office</a></dd>
        <dt className="text-ink-3">Licences</dt>
        <dd className="m-0">
          <a href="#third-party-licenses" aria-disabled={!noticesAvailable || undefined} onClick={(e) => { e.preventDefault(); if (noticesAvailable) onOpenLink?.('licenses') }} className="text-ink underline">Third-party licenses</a>
          <span className="text-ink-3"> (LibreOffice MPL-2.0, Qt LGPL-3.0, fonts OFL-1.1)</span>
        </dd>
      </dl>
    </Modal>
  )
}

const MB = 1024 * 1024

/** The first open: the editor downloads and starts on this PC (DW-1555). A skeleton of the page under it. */
export function PreparingView({ loaded, total }: { loaded: number; total: number }) {
  const pct = total > 0 ? Math.min(100, Math.round((loaded / total) * 100)) : 0
  return (
    <div data-preparing className="relative grid min-h-0 flex-1 place-items-center overflow-hidden bg-paper-3">
      <div aria-hidden="true" className="absolute top-6 flex h-[700px] w-[520px] flex-col gap-3 rounded-sm bg-paper p-12 opacity-70 shadow">
        <span className="h-5 w-48 rounded bg-paper-3" /><span className="h-3 w-72 rounded bg-paper-3" />
        {Array.from({ length: 9 }, (_, i) => <span key={i} className="h-3 rounded bg-paper-3" style={{ width: `${70 + ((i * 37) % 30)}%` }} />)}
      </div>
      <div role="status" aria-live="polite" className="relative w-[340px] rounded-xl border border-line bg-paper p-5 text-center shadow-xl">
        <p className="m-0 text-[14px] font-medium text-ink">Preparing the editor on this PC…</p>
        <p className="m-0 mt-1 font-mono text-[12px] text-ink-3">{Math.round(loaded / MB)} of {Math.round(total / MB)} MB</p>
        <div role="progressbar" aria-label="Preparing the editor" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} className="mt-3 h-1.5 overflow-hidden rounded-full bg-paper-3">
          <div className="h-full rounded-full bg-ink-2" style={{ width: `${pct}%` }} />
        </div>
        <p className="m-0 mt-3 text-[12px] text-ink-3">After the first time the editor is on this PC and starts in a second.</p>
      </div>
    </div>
  )
}

/** The editor didn't start (DW-1556): said plainly, with what still works. */
export function FailedView({ detail, onRetry, onOtherApp }: { detail?: string; onRetry: () => void; onOtherApp: () => void }) {
  return (
    <div data-failed className="grid min-h-0 flex-1 place-items-center bg-paper-3">
      <div role="alert" className="w-[400px] rounded-xl border border-line bg-paper p-6 text-center shadow-xl">
        <p className="m-0 text-[15px] font-semibold text-ink">We couldn’t prepare the editor on this device.</p>
        <p className="m-0 mt-2 text-[13px] text-ink-2">The file is untouched; it can still be read in Preview.</p>
        {detail && <p className="m-0 mt-2 font-mono text-[11.5px] text-ink-3">{detail}</p>}
        <div className="mt-4 flex justify-center gap-2">
          <button type="button" onClick={onRetry} className={BTN}>Try again</button>
          <button type="button" onClick={onOtherApp} className={BTN}>Open in another app…</button>
        </div>
      </div>
    </div>
  )
}

const LEGACY: Record<string, string> = { doc: 'An older Word file (.doc)', xls: 'An older Excel file (.xls)', ppt: 'An older PowerPoint file (.ppt)' }
export const isLegacy = (ext: string) => ext in LEGACY

/** Older Word, Excel and PowerPoint files open too (DW-1511); the note is the only difference. */
export function LegacyNote({ ext, onOtherApp }: { ext: string; onOtherApp?: () => void }) {
  return (
    <div role="note" data-legacy-note className="flex h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 text-[13px] text-ink">
      <Icon name="info" size={14} />
      <span className="flex-1">{LEGACY[ext]}. It opens and saves in the same format.</span>
      <a href="#open-with" onClick={(e) => { e.preventDefault(); onOtherApp?.() }} className="text-ink underline">Open in another app…</a>
    </div>
  )
}

const KIND_LABEL: Record<OfficeApp, string> = { writer: 'Docs', calc: 'Sheets', impress: 'Slides' }
const DEFAULT_NAME: Record<OfficeApp, string> = { writer: 'Untitled document.docx', calc: 'Untitled spreadsheet.xlsx', impress: 'Untitled presentation.pptx' }
const SHEET_TEMPLATES: TemplateDef[] = [{ id: 'blank', label: 'Blank', description: 'Start from nothing' }]

/** "New document" (DW-1558): Docs · Sheets · Slides, the web editor's templates, a name, Create. */
export function NewDocumentDialog({ kind, template, name, onKind, onTemplate, onName, onCreate, onCancel, busy, error }: {
  kind: OfficeApp; template: TemplateDef['id']; name: string; busy?: boolean; error?: string | null
  onKind: (k: OfficeApp) => void; onTemplate: (t: TemplateDef['id']) => void; onName: (n: string) => void; onCreate: () => void; onCancel: () => void
}) {
  const templates = kind === 'writer' ? TEMPLATE_DEFS : SHEET_TEMPLATES
  return (
    <Modal title="New document" label="New document" onClose={onCancel} width={560}
      footer={<><span className="flex-1" /><button type="button" onClick={onCancel} className={BTN}>Cancel</button><button type="button" onClick={onCreate} disabled={busy || !name.trim()} className={`${BTN_AMBER} disabled:opacity-50`}>{busy ? 'Creating…' : 'Create'}</button></>}>
      <div role="tablist" aria-label="Kind" className="flex gap-1">
        {(['writer', 'calc', 'impress'] as OfficeApp[]).map((k) => (
          <button key={k} type="button" role="tab" aria-selected={k === kind} onClick={() => { onKind(k); onName(DEFAULT_NAME[k]) }}
            className={`h-7 rounded-md px-3 text-[13px] ${k === kind ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2 hover:bg-paper-2'} ${FOCUS}`}>{KIND_LABEL[k]}</button>
        ))}
      </div>
      <div role="radiogroup" aria-label="Templates" className="mt-3 grid grid-cols-3 gap-2">
        {templates.map((t) => (
          <button key={t.id} type="button" role="radio" aria-checked={t.id === template} data-template={t.label} onClick={() => onTemplate(t.id)}
            className={`flex flex-col items-start gap-0.5 rounded-lg border p-3 text-left ${t.id === template ? 'border-ink-3 bg-paper-3' : 'border-line hover:bg-paper-2'} ${FOCUS}`}>
            <span className="text-[13px] font-medium text-ink">{t.label}</span>
            <span className="text-[12px] text-ink-3">{t.description}</span>
          </button>
        ))}
      </div>
      <label className="mt-4 block text-[12px] font-medium text-ink-2" htmlFor="office-new-name">Name</label>
      <input id="office-new-name" value={name} onChange={(e) => onName(e.target.value)} className="mt-1 h-8 w-full rounded-md border border-line bg-paper px-2 text-[13px] text-ink outline-none focus:border-ink-3" />
      {error && <p role="alert" className="m-0 mt-1 text-[12px] text-ink">{error}</p>}
      <p className="m-0 mt-3 flex items-center gap-1.5 text-[12px] text-ink-3"><span className="text-amber-deep"><Icon name="lock" size={11} /></span>Everything you create here is encrypted before it leaves this device.</p>
    </Modal>
  )
}
export const defaultNewName = (k: OfficeApp) => DEFAULT_NAME[k]

/** Rows and columns, then inserts (DW-1550's Table…). */
export function TableSizeDialog({ onInsert, onCancel }: { onInsert: (rows: number, cols: number) => void; onCancel: () => void }) {
  const [rows, setRows] = useState(3)
  const [cols, setCols] = useState(3)
  const field = (label: string, v: number, set: (n: number) => void) => (
    <label className="flex items-center gap-2 text-[13px]">{label}
      <input type="number" min={1} max={50} value={v} onChange={(e) => set(Math.max(1, Math.min(50, Number(e.target.value) || 1)))} className="h-8 w-16 rounded-md border border-line bg-paper px-2 font-mono text-[12.5px] text-ink" />
    </label>
  )
  return (
    <Modal title="Insert table" onClose={onCancel} width={340} footer={<><span className="flex-1" /><button type="button" onClick={onCancel} className={BTN}>Cancel</button><button type="button" onClick={() => onInsert(rows, cols)} className={BTN}>Insert</button></>}>
      <div className="flex gap-4">{field('Rows', rows, setRows)}{field('Columns', cols, setCols)}</div>
    </Modal>
  )
}
