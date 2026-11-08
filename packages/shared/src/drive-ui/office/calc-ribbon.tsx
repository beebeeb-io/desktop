/**
 * CalcRibbon (task 1567, Calc lane) -- the Home/Insert/Data/Formulas ribbon
 * for a Calc document, matching design/office-editor-shots/calc-*.png.
 *
 * Its own component, not a branch inside `./ribbon.tsx`: that file's
 * `GENERIC_TABS` fallback stays exactly as the core lane left it for
 * whichever apps still want it, and this lane never needs to touch a file
 * other office lanes are concurrently extending. `office-editor.tsx` renders
 * this INSTEAD OF `<Ribbon/>` when `officeApp === 'calc'` (the one
 * registration-hook line this lane's brief allows).
 *
 * Only the Home tab has real content -- Insert/Data/Formulas are clickable
 * but render an empty ribbon row, same precedent `./ribbon.tsx` already
 * established for Writer's own Insert/Layout/Review tabs (the approved
 * mockup itself only shows Home populated for every app).
 */

import {
  CALC_HOME_COMMANDS,
  deriveCalcButtonState,
  type CalcCommandDef,
  type UnoStateMap,
} from '../lib/office/calc-commands'
import { IconAlignCenter, IconAlignLeft, IconAlignRight, IconChevronDown, IconRedo, IconTable, IconUndo } from './office-icons'
import { IconFillSwatch, IconFilter, IconSort } from './calc-icons'

const CALC_TABS = ['Home', 'Insert', 'Data', 'Formulas']

// CRITIQUE.md finding #9 (task 1567): 'autosum' was referenced by byId()
// below but had NO entry in either glyph map below it or in SVG_ICON --
// CalcRibbonButton's render falls through to `null` when neither map has the
// id, so the button existed (clickable, correctly enabled/disabled) but drew
// nothing inside it. Sigma as a plain text glyph, matching this file's own
// existing precedent for a small set of characters (B/I/U/.00) rather than a
// new SVG asset.
const GLYPH_ICON: Record<string, string> = {
  bold: 'B',
  italic: 'I',
  underline: 'U',
  'inc-decimals': '.00',
  autosum: 'Σ',
}
const GLYPH_STYLE: Record<string, React.CSSProperties> = {
  bold: { fontWeight: 700 },
  italic: { fontStyle: 'italic' },
  underline: { textDecoration: 'underline' },
  'inc-decimals': { fontSize: '11px', fontFamily: 'var(--font-mono)' },
  autosum: { fontSize: '15px', fontWeight: 600 },
}
const SVG_ICON: Record<string, (p: { size?: number; className?: string }) => React.ReactElement> = {
  undo: IconUndo,
  redo: IconRedo,
  'align-left': IconAlignLeft,
  'align-center': IconAlignCenter,
  'align-right': IconAlignRight,
  sort: IconSort,
  filter: IconFilter,
}

function byId(id: string): CalcCommandDef {
  const def = CALC_HOME_COMMANDS.find((d) => d.id === id)
  if (!def) throw new Error(`calc-ribbon: no command def for id "${id}"`)
  return def
}

function CalcRibbonButton({
  def,
  states,
  onCommand,
  disabled,
  title,
}: {
  def: CalcCommandDef
  states: UnoStateMap
  onCommand: (def: CalcCommandDef) => void
  disabled?: boolean
  title?: string
}) {
  const { enabled, pressed } = deriveCalcButtonState(states, def.command)
  const isDisabled = disabled || !enabled
  const glyph = GLYPH_ICON[def.id]
  const SvgIcon = SVG_ICON[def.id]
  return (
    <button
      type="button"
      disabled={isDisabled}
      aria-label={def.label}
      aria-pressed={def.action ? undefined : pressed}
      data-testid={`calc-ribbon-${def.id}`}
      title={title ?? (def.shortcut ? `${def.label} (${def.shortcut})` : def.label)}
      onClick={() => onCommand(def)}
      className={`grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md transition-colors disabled:opacity-40 ${
        pressed ? 'bg-amber-bg text-ink shadow-[inset_0_-2px_0_var(--color-amber)]' : 'text-ink hover:bg-paper-3'
      }`}
    >
      {glyph ? (
        <span className="font-sans text-[14px]" style={GLYPH_STYLE[def.id]}>
          {glyph}
        </span>
      ) : SvgIcon ? (
        <SvgIcon size={16} />
      ) : null}
    </button>
  )
}

function Divider() {
  return <div className="h-[22px] w-px shrink-0 bg-line" />
}

export interface CalcRibbonProps {
  activeTab: string
  onTabChange: (tab: string) => void
  states: UnoStateMap
  onCommand: (def: CalcCommandDef) => void
}

export function CalcRibbon({ activeTab, onTabChange, states, onCommand }: CalcRibbonProps) {
  const showHomeControls = activeTab === 'Home'

  return (
    <div className="flex h-12 shrink-0 items-center gap-3.5 border-b border-line bg-paper-2 px-4" data-testid="office-ribbon">
      <div className="flex h-full shrink-0 items-center gap-0.5">
        {CALC_TABS.map((tab) => (
          <button
            key={tab}
            type="button"
            onClick={() => onTabChange(tab)}
            aria-current={activeTab === tab}
            data-testid={`ribbon-tab-${tab.toLowerCase()}`}
            className={`relative flex h-full items-center whitespace-nowrap px-3 text-[13px] ${
              activeTab === tab ? 'font-semibold text-ink' : 'font-medium text-ink-3'
            }`}
          >
            {tab}
            {activeTab === tab && <span className="absolute inset-x-[9px] bottom-0 h-[2px] rounded-t-sm bg-amber" />}
          </button>
        ))}
      </div>
      <Divider />
      {showHomeControls && (
        <div className="flex min-w-0 flex-1 items-center gap-2.5 overflow-x-auto">
          <div className="flex shrink-0 items-center gap-0.5">
            <CalcRibbonButton def={byId('undo')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('redo')} states={states} onCommand={onCommand} />
          </div>
          <Divider />
          {/* Font name/size: decorative-only, same treatment the Writer
              ribbon already gives its "Normal text" style pill (no
              onState/dispatch exists for these in this build's bridge). */}
          <div className="flex shrink-0 items-center gap-1.5">
            <div className="flex h-[30px] min-w-[86px] items-center justify-between gap-1.5 rounded-md border border-line bg-paper px-2 text-[12.5px] text-ink">
              <span>Inter</span>
              <IconChevronDown size={11} className="text-ink-3" />
            </div>
            <div className="flex h-[30px] w-[52px] items-center justify-between gap-1 rounded-md border border-line bg-paper px-2 text-[12.5px] text-ink">
              <span>10</span>
              <IconChevronDown size={11} className="text-ink-3" />
            </div>
          </div>
          <Divider />
          <div className="flex shrink-0 items-center gap-0.5">
            <CalcRibbonButton def={byId('bold')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('italic')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('underline')} states={states} onCommand={onCommand} />
          </div>
          <Divider />
          <div className="flex shrink-0 items-center gap-0.5">
            {/* `.uno:InsertTable` has no dispatch handler in this engine
                build (verified) -- honestly disabled rather than wired to a
                command that silently does nothing. */}
            <button
              type="button"
              disabled
              aria-label="Insert table"
              title="Not available in this editor yet"
              data-testid="calc-ribbon-table"
              className="grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md text-ink opacity-40"
            >
              <IconTable size={16} />
            </button>
            <button
              type="button"
              disabled
              aria-label="Fill color"
              title="Not available in this editor yet"
              data-testid="calc-ribbon-fill"
              className="grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md text-ink opacity-40"
            >
              <IconFillSwatch size={15} />
            </button>
          </div>
          <Divider />
          <div className="flex shrink-0 items-center gap-0.5">
            <CalcRibbonButton def={byId('align-left')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('align-center')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('align-right')} states={states} onCommand={onCommand} />
          </div>
          <Divider />
          <div className="flex shrink-0 items-center gap-1.5">
            <button
              type="button"
              onClick={() => onCommand(byId('currency'))}
              aria-pressed={deriveCalcButtonState(states, '.uno:NumberFormatCurrency').pressed}
              data-testid="calc-ribbon-currency"
              title="Currency format"
              className={`flex h-[30px] min-w-[92px] items-center justify-between gap-1.5 rounded-md border border-line px-2 text-[12.5px] ${
                deriveCalcButtonState(states, '.uno:NumberFormatCurrency').pressed ? 'bg-amber-bg text-ink' : 'bg-paper text-ink'
              }`}
            >
              <span>€ Currency</span>
              <IconChevronDown size={11} className="text-ink-3" />
            </button>
            <CalcRibbonButton def={byId('inc-decimals')} states={states} onCommand={onCommand} title=".00" />
          </div>
          <Divider />
          <div className="flex shrink-0 items-center gap-0.5">
            <CalcRibbonButton def={byId('autosum')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('sort')} states={states} onCommand={onCommand} />
            <CalcRibbonButton def={byId('filter')} states={states} onCommand={onCommand} />
          </div>
        </div>
      )}
      {!showHomeControls && <div className="flex-1" />}
    </div>
  )
}
