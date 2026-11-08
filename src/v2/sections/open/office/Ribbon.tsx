// The Office ribbon under the window's toolbar (DW-1507, DW-1550, DW-1508, DW-1559, DW-1509, DW-1560): tabs, the
// Ctrl+K search pill, Share and Save, then the active tab's controls. The active tab is underlined in ink, never
// amber (task 2321); Save is a plain button (New stays the window's one amber control).
import type { ReactNode } from 'react'
import { Icon, type IconName } from '@beebeeb/shared/design'
import { IMPRESS_LAYOUTS, type OfficeApp } from '@beebeeb/shared/drive-ui'
import { BTN, FOCUS } from '../../files/ui'

export const OFFICE_TABS: Record<OfficeApp, string[]> = {
  writer: ['Home', 'Insert', 'Layout', 'Review'],
  calc: ['Home', 'Insert', 'Data', 'Formulas'],
  impress: ['Home', 'Insert', 'Design', 'Transitions'],
}

const GLYPHS: Record<string, string[]> = {
  undo: ['M9 14L4 9l5-5', 'M4 9h10.5a5.5 5.5 0 0 1 0 11H11'],
  redo: ['M15 14l5-5-5-5', 'M20 9H9.5a5.5 5.5 0 0 0 0 11H13'],
  bullets: ['M9 6h11', 'M9 12h11', 'M9 18h11', 'M4.5 6h.01', 'M4.5 12h.01', 'M4.5 18h.01'],
  numbers: ['M10 6h10', 'M10 12h10', 'M10 18h10', 'M4 5h1.5v4', 'M4 15.5a1.5 1.5 0 0 1 3 0c0 1.5-3 2.5-3 4h3'],
  alignL: ['M4 6h16', 'M4 10h10', 'M4 14h16', 'M4 18h10'],
  alignC: ['M4 6h16', 'M7 10h10', 'M4 14h16', 'M7 18h10'],
  alignR: ['M4 6h16', 'M10 10h10', 'M4 14h16', 'M10 18h10'],
  justify: ['M4 6h16', 'M4 10h16', 'M4 14h16', 'M4 18h16'],
  indentL: ['M11 6h9', 'M11 12h9', 'M11 18h9', 'M7 9l-3 3 3 3'],
  indentR: ['M11 6h9', 'M11 12h9', 'M11 18h9', 'M4 9l3 3-3 3'],
  table: ['M4.5 4h15A1.5 1.5 0 0 1 21 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-13A1.5 1.5 0 0 1 4.5 4z', 'M3 10h18', 'M3 15h18', 'M9 4v16', 'M15 4v16'],
  color: ['M6 18l6-14 6 14', 'M8.5 12.5h7', 'M4 21h16'],
  marker: ['M15 4l5 5-9 9H6v-5z', 'M4 21h16'],
  sum: ['M17 5H7l6 7-6 7h10'],
  filter: ['M4 5h16l-6 7v6l-4 2v-8z'],
  sortAZ: ['M4 18l3-9 3 9', 'M5 15h4', 'M14 6h6l-6 9h6', 'M17 18v3'],
  present: ['M4 5h16v11H4z', 'M12 16v4', 'M9 20h6', 'M10.5 8.5v4l3.5-2z'],
  slide: ['M4 6h16v12H4z', 'M12 9v6', 'M9 12h6'],
  fx: ['M9 5H7.5A1.5 1.5 0 0 0 6 6.5V19', 'M4 11h5', 'M13 9l6 8', 'M19 9l-6 8'],
  pagebreak: ['M6 3v6h12V3', 'M6 21v-6h12v6', 'M3 12h3', 'M9 12h2', 'M13 12h2', 'M18 12h3'],
}

export function Glyph({ name, size = 15 }: { name: string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {(GLYPHS[name] ?? []).map((d) => <path key={d} d={d} />)}
    </svg>
  )
}

/** One ribbon control. `uno` is the engine command it runs; `text` shows a label beside the glyph. */
export interface Control {
  id: string
  label: string
  uno?: string
  glyph?: string
  icon?: IconName
  letter?: { text: string; style?: React.CSSProperties }
  text?: string
  shortcut?: string
  on?: boolean
  off?: boolean
  select?: { value: string; width: number }
  sep?: boolean
}

const SEP: Control = { id: 'sep', label: '', sep: true }
const sep = (n: number): Control => ({ ...SEP, id: `sep-${n}` })
const UNDO: Control[] = [
  { id: 'undo', label: 'Undo', uno: '.uno:Undo', glyph: 'undo', shortcut: 'Ctrl+Z' },
  { id: 'redo', label: 'Redo', uno: '.uno:Redo', glyph: 'redo', shortcut: 'Ctrl+Y', off: true },
]
const FONT: Control[] = [
  { id: 'font', label: 'Font', select: { value: 'Inter', width: 92 } },
  { id: 'size', label: 'Font size', select: { value: '11', width: 48 } },
]
const BIU: Control[] = [
  { id: 'bold', label: 'Bold', uno: '.uno:Bold', letter: { text: 'B', style: { fontWeight: 800 } }, shortcut: 'Ctrl+B' },
  { id: 'italic', label: 'Italic', uno: '.uno:Italic', letter: { text: 'I', style: { fontStyle: 'italic', fontFamily: 'serif' } }, shortcut: 'Ctrl+I' },
  { id: 'underline', label: 'Underline', uno: '.uno:Underline', letter: { text: 'U', style: { textDecoration: 'underline' } }, shortcut: 'Ctrl+U' },
]

/** The controls of each app's tab, as the board draws them. */
export function ribbonControls(app: OfficeApp, tab: string): Control[] {
  if (app === 'writer' && tab === 'Insert') return [
    { id: 'insert-table', label: 'Insert table…', glyph: 'table', text: 'Table…' },
    { id: 'insert-image', label: 'Insert image', icon: 'image', text: 'Image…' },
    { id: 'insert-link', label: 'Insert link', icon: 'link', text: 'Link…' },
    sep(1),
    { id: 'page-break', label: 'Page break', uno: '.uno:InsertPagebreak', glyph: 'pagebreak', text: 'Page break' },
  ]
  if (app === 'writer' && tab === 'Layout') return [{ id: 'page-layout', label: 'Page layout…', uno: '.uno:PageDialog', icon: 'file-text', text: 'Page layout…' }]
  if (app === 'writer' && tab === 'Review') return [{ id: 'word-count', label: 'Word count…', uno: '.uno:WordCountDialog', icon: 'file-text', text: 'Word count…' }]
  if (app === 'writer') return [
    ...UNDO, sep(1),
    { id: 'style', label: 'Paragraph style', select: { value: 'Normal text', width: 112 } }, ...FONT, sep(2),
    ...BIU,
    { id: 'strike', label: 'Strikethrough', uno: '.uno:Strikeout', letter: { text: 'S', style: { textDecoration: 'line-through' } } },
    { id: 'text-colour', label: 'Text colour', uno: '.uno:Color', glyph: 'color' },
    { id: 'highlight', label: 'Highlight', uno: '.uno:CharBackColor', glyph: 'marker' }, sep(3),
    { id: 'bullets', label: 'Bulleted list', uno: '.uno:DefaultBullet', glyph: 'bullets' },
    { id: 'numbers', label: 'Numbered list', uno: '.uno:DefaultNumbering', glyph: 'numbers' }, sep(4),
    { id: 'align-left', label: 'Align left', uno: '.uno:LeftPara', glyph: 'alignL', shortcut: 'Ctrl+L', on: true },
    { id: 'align-center', label: 'Align center', uno: '.uno:CenterPara', glyph: 'alignC', shortcut: 'Ctrl+E' },
    { id: 'align-right', label: 'Align right', uno: '.uno:RightPara', glyph: 'alignR', shortcut: 'Ctrl+R' },
    { id: 'justify', label: 'Justify', uno: '.uno:JustifyPara', glyph: 'justify', shortcut: 'Ctrl+J' }, sep(5),
    { id: 'indent-less', label: 'Decrease indent', uno: '.uno:DecrementIndent', glyph: 'indentL' },
    { id: 'indent-more', label: 'Increase indent', uno: '.uno:IncrementIndent', glyph: 'indentR' }, sep(6),
    { id: 'insert-link', label: 'Insert link', icon: 'link' },
    { id: 'insert-image', label: 'Insert image', icon: 'image' },
    { id: 'insert-table', label: 'Insert table', glyph: 'table' },
  ]
  if (app === 'calc' && tab === 'Data') return [
    { id: 'sort-az', label: 'Sort ascending', uno: '.uno:SortAscending', glyph: 'sortAZ', text: 'Sort A to Z' },
    { id: 'filter', label: 'AutoFilter', uno: '.uno:DataFilterAutoFilter', glyph: 'filter', text: 'Filter' }, sep(1),
    { id: 'table', label: 'Insert table', uno: '.uno:DefineDBName', glyph: 'table', text: 'Table' },
  ]
  if (app === 'calc' && tab === 'Insert') return [
    { id: 'insert-image', label: 'Insert image', icon: 'image', text: 'Image…' },
    { id: 'insert-link', label: 'Insert link', icon: 'link', text: 'Link…' },
  ]
  if (app === 'calc' && tab === 'Formulas') return [{ id: 'sum', label: 'Sum', uno: '.uno:AutoSum', glyph: 'sum', shortcut: 'Alt+=', text: 'Sum' }]
  if (app === 'calc') return [
    ...UNDO, sep(1), ...FONT, sep(2), ...BIU, sep(3),
    { id: 'align-left', label: 'Align left', uno: '.uno:AlignLeft', glyph: 'alignL' },
    { id: 'align-center', label: 'Align center', uno: '.uno:AlignHorizontalCenter', glyph: 'alignC' },
    { id: 'align-right', label: 'Align right', uno: '.uno:AlignRight', glyph: 'alignR', on: true }, sep(4),
    { id: 'currency', label: 'Currency format', uno: '.uno:NumberFormatCurrency', letter: { text: '€' } },
    { id: 'decimal', label: 'Add decimal place', uno: '.uno:NumberFormatIncDecimals', letter: { text: '.00', style: { fontFamily: 'var(--font-mono)', fontSize: 11 } } },
    { id: 'percent', label: 'Number format: Percent', uno: '.uno:NumberFormatPercent', letter: { text: '%' } },
    { id: 'fill', label: 'Fill colour', uno: '.uno:BackgroundColor', glyph: 'color' }, sep(5),
    { id: 'sum', label: 'Sum', uno: '.uno:AutoSum', glyph: 'sum', shortcut: 'Alt+=', text: 'Sum' },
    { id: 'sort', label: 'Sort ascending', uno: '.uno:SortAscending', glyph: 'sortAZ' },
    { id: 'autofilter', label: 'AutoFilter', uno: '.uno:DataFilterAutoFilter', glyph: 'filter' },
  ]
  if (app === 'impress' && tab === 'Design') return []
  if (app === 'impress' && tab === 'Insert') return [
    { id: 'insert-image', label: 'Insert image', icon: 'image', text: 'Image…' },
    { id: 'new-slide', label: 'New slide', uno: '.uno:InsertPage', glyph: 'slide', shortcut: 'Ctrl+M', text: 'New slide' },
  ]
  if (app === 'impress' && tab === 'Transitions') return [{ id: 'present', label: 'Present', glyph: 'present', shortcut: 'Ctrl+Enter', text: 'Present' }]
  return [
    ...UNDO, sep(1), ...FONT, sep(2), ...BIU,
    { id: 'align-center', label: 'Align center', uno: '.uno:CenterPara', glyph: 'alignC', shortcut: 'Ctrl+E' }, sep(3),
    { id: 'new-slide', label: 'New slide', uno: '.uno:InsertPage', glyph: 'slide', shortcut: 'Ctrl+M', text: 'New slide' },
    { id: 'duplicate-slide', label: 'Duplicate slide', uno: '.uno:DuplicatePage', icon: 'copy' },
    { id: 'delete-slide', label: 'Delete slide', uno: '.uno:DeletePage', icon: 'trash' }, sep(4),
    { id: 'insert-image', label: 'Insert image', icon: 'image' }, sep(5),
    { id: 'present', label: 'Present', glyph: 'present', shortcut: 'Ctrl+Enter', text: 'Present' },
  ]
}

const TBTN = `inline-flex h-7 flex-none items-center gap-1.5 rounded-md px-1.5 text-ink hover:bg-paper-3 disabled:opacity-40 disabled:hover:bg-transparent ${FOCUS}`

function ControlButton({ c, onRun }: { c: Control; onRun: (c: Control) => void }) {
  if (c.sep) return <span aria-hidden="true" className="mx-1 h-5 w-px flex-none bg-line" />
  if (c.select) {
    return (
      <button type="button" aria-label={c.label} title={c.label} onClick={() => onRun(c)} style={{ minWidth: c.select.width }} className={`inline-flex h-6 flex-none items-center gap-1 rounded-md border border-line bg-paper px-2 text-[12px] text-ink ${FOCUS}`}>
        <span className="flex-1 truncate text-left">{c.select.value}</span><Icon name="chevron-down" size={11} />
      </button>
    )
  }
  return (
    <button
      type="button" data-control={c.id} aria-label={c.label} title={c.shortcut ? `${c.label}  ${c.shortcut}` : c.label} aria-pressed={c.on ? true : undefined} disabled={c.off} onClick={() => onRun(c)}
      className={`${TBTN} ${c.on ? 'bg-paper-3 shadow-[inset_0_0_0_1px_var(--color-line)]' : ''}`}
    >
      {c.letter ? <b className="w-[15px] text-center text-[13.5px]" style={c.letter.style}>{c.letter.text}</b> : c.glyph ? <Glyph name={c.glyph} /> : c.icon ? <Icon name={c.icon} size={15} /> : null}
      {c.text && <span className="text-[12.5px]">{c.text}</span>}
    </button>
  )
}

/** Design's layouts as one-line chips: a small sketch, then the name (DW-1560). */
export function LayoutChips({ current, onPick }: { current: string; onPick: (id: string) => void }) {
  const bars: [number, number, number][][] = [[[6, 6, 16], [9, 10, 10]], [[4, 3, 14], [4, 8, 20], [4, 11, 20], [4, 14, 14]], [[4, 3, 14], [4, 8, 9], [4, 11, 9], [15, 8, 9], [15, 11, 9]], [[4, 3, 14]], []]
  return (
    <div role="radiogroup" aria-label="Layouts" className="flex items-center gap-1">
      {IMPRESS_LAYOUTS.map((l, i) => (
        <button key={l.id} type="button" role="radio" aria-checked={l.id === current} data-layout={l.label} onClick={() => onPick(l.id)}
          className={`flex flex-none items-center gap-1.5 rounded-[7px] px-1.5 py-1 hover:bg-paper-3 ${l.id === current ? 'bg-paper-3 shadow-[inset_0_0_0_1px_var(--color-ink-3)]' : ''} ${FOCUS}`}>
          <span className="relative block h-[18px] w-[28px] flex-none rounded-[2px] bg-paper shadow-[inset_0_0_0_0.5px_var(--color-line)]">
            {bars[i].map(([x, y, w], j) => <i key={j} className="absolute rounded-[1px] bg-ink-3" style={{ left: x, top: y, width: w, height: j === 0 ? 2.5 : 2 }} />)}
          </span>
          <span className="whitespace-nowrap text-[11px] text-ink-2">{l.label}{l.id === current ? ' (current)' : ''}</span>
        </button>
      ))}
    </div>
  )
}

export interface RibbonProps {
  app: OfficeApp
  tab: string
  onTab: (tab: string) => void
  onRun: (c: Control) => void
  dirty: boolean
  saving: boolean
  onSave: () => void
  onPalette: () => void
  onShare?: () => void
  /** Preparing the editor: everything dimmed and inert (DW-1555). */
  dimmed?: boolean
  layout?: { current: string; onPick: (id: string) => void }
  children?: ReactNode
}

export function Ribbon({ app, tab, onTab, onRun, dirty, saving, onSave, onPalette, onShare, dimmed, layout }: RibbonProps) {
  const controls = ribbonControls(app, tab)
  return (
    <div data-ribbon={app} aria-disabled={dimmed || undefined} className={`flex flex-none flex-col border-b border-line bg-paper-2 ${dimmed ? 'pointer-events-none opacity-50' : ''}`}>
      <div className="flex h-8 items-center gap-1 px-3">
        <div role="tablist" aria-label="Ribbon" className="flex h-full items-center">
          {OFFICE_TABS[app].map((t) => (
            <button key={t} type="button" role="tab" aria-selected={t === tab} onClick={() => onTab(t)}
              className={`relative grid h-full place-items-center px-2.5 text-[12.5px] ${t === tab ? 'font-semibold text-ink' : 'font-medium text-ink-3 hover:text-ink'} ${FOCUS}`}>
              {t}
              {t === tab && <span className="absolute inset-x-2 bottom-0 h-[2px] rounded-t-sm bg-ink" />}
            </button>
          ))}
        </div>
        <span className="flex-1" />
        <button type="button" onClick={onPalette} data-palette-pill className={`inline-flex h-6 items-center gap-1.5 rounded-full border border-line bg-paper px-2.5 text-[11.5px] text-ink-3 hover:bg-paper-3 ${FOCUS}`}>
          <Icon name="search" size={12} />Search commands, styles, files…<kbd className="font-mono text-[10.5px] text-ink-3">Ctrl+K</kbd>
        </button>
        <button type="button" onClick={onShare} className={`${BTN} h-7 gap-1.5 px-3 text-[12.5px]`}><Icon name="share" size={13} />Share</button>
        <button type="button" onClick={onSave} disabled={!dirty || saving} data-save className={`${BTN} h-7 gap-1.5 px-3 text-[12.5px] disabled:opacity-45`}>
          <Icon name="lock" size={13} />{saving ? 'Saving…' : 'Save'}<span className="ml-1 font-mono text-[10.5px] font-normal text-ink-3">Ctrl+S</span>
        </button>
      </div>
      <div role="toolbar" aria-label={`${tab} controls`} className="flex h-[38px] items-center gap-0.5 overflow-hidden border-t border-line bg-paper px-2">
        {app === 'impress' && tab === 'Design' && layout ? <LayoutChips current={layout.current} onPick={layout.onPick} /> : controls.map((c) => <ControlButton key={c.id} c={c} onRun={onRun} />)}
      </div>
    </div>
  )
}
