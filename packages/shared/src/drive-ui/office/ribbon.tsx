/**
 * Ribbon (task 1567) — the one-row, contextual-tab toolbar that replaces the
 * engine's own (hidden) toolbars, per PLAN.md's hybrid architecture: React
 * draws every button, the engine's `bbOffice.dispatch()`/`onState()` is the
 * only thing that ever runs a command or reports whether it's active.
 *
 * `WRITER_HOME_COMMANDS` + `deriveButtonState` (../../lib/office/ribbon-commands)
 * are the pure, unit-tested halves this component wires up to the live
 * engine — this file itself owns no command logic, only the DOM.
 */

import type { OfficeApp } from '../lib/office/office-file-kind'
import {
  WRITER_HOME_COMMANDS,
  WRITER_PARAGRAPH_STYLES,
  deriveButtonState,
  type RibbonCommandDef,
  type UnoStateMap,
} from '../lib/office/ribbon-commands'
import type { LayoutDef, SlideOp } from '../lib/office/impress-commands'
import { ImpressRibbonContent, IMPRESS_TABS, type ImpressTab } from './impress-ribbon'
import { FontPickers, ColorPickers, type CharFormattingState } from './char-formatting-controls'
import {
  IconAlignCenter,
  IconAlignJustify,
  IconAlignLeft,
  IconAlignRight,
  IconChevronDown,
  IconIndentLess,
  IconIndentMore,
  IconListBullet,
  IconListNumbered,
  IconRedo,
  IconTable,
  IconUndo,
} from './office-icons'
import { Icon } from '../../design'

const GLYPH_ICON: Record<string, string> = {
  bold: 'B',
  italic: 'I',
  underline: 'U',
  strikethrough: 'S',
}
const GLYPH_STYLE: Record<string, React.CSSProperties> = {
  bold: { fontWeight: 700 },
  italic: { fontStyle: 'italic' },
  underline: { textDecoration: 'underline' },
  strikethrough: { textDecoration: 'line-through' },
}
const SVG_ICON: Record<string, (p: { size?: number; className?: string }) => React.ReactElement> = {
  undo: IconUndo,
  redo: IconRedo,
  'align-left': IconAlignLeft,
  'align-center': IconAlignCenter,
  'align-right': IconAlignRight,
  'align-justify': IconAlignJustify,
  'bullet-list': IconListBullet,
  'numbered-list': IconListNumbered,
  'indent-more': IconIndentMore,
  'indent-less': IconIndentLess,
}

export interface RibbonProps {
  app: OfficeApp
  activeTab: string
  onTabChange: (tab: string) => void
  states: UnoStateMap
  onCommand: (def: RibbonCommandDef) => void
  onInsertLink: () => void
  onInsertImage: () => void
  /** CRITIQUE.md finding #4 (task 1567): Writer-only, matching the mockup's
   *  own "font + size, B/I/U/S, text colour, highlight" Home-tab order.
   *  Undefined for Calc/Impress, whose ribbons don't render these controls
   *  (Calc/Impress own char-formatting UI is a separate, unscoped follow-up). */
  charFormatting?: {
    state: CharFormattingState
    onFontName: (name: string) => void
    onFontHeight: (points: number) => void
    onTextColor: (unoColor: number) => void
    onHighlightColor: (unoColor: number) => void
  }
  /** Impress-only extras (task 1567 Impress lane) — undefined for Writer/
   *  Calc, whose ribbon content never reads this prop. Bundled rather than
   *  spread so this lane's additions are one optional prop, not a change to
   *  every existing call site's positional/required props. */
  impress?: {
    states: UnoStateMap
    busy: boolean
    slideCount: number
    onSlideOp: (op: SlideOp) => void
    onApplyLayout: (layout: LayoutDef) => void
    onPresent: () => void
  }
}

const WRITER_TABS = ['Home', 'Insert', 'Layout', 'Review']
/** Generic ribbon for Calc until its own lane ships a real one (task brief:
 *  "routed to the same editor, even if their ribbons come later") —
 *  Bold/Italic/Underline + Undo/Redo cover the common case so opening an
 *  .xlsx here is never a dead end. Impress has its own real tab set
 *  (IMPRESS_TABS, imported above) — see the `app === 'impress'` branch
 *  below. */
const GENERIC_TABS = ['Home']

function RibbonButton({
  def,
  states,
  onCommand,
}: {
  def: RibbonCommandDef
  states: UnoStateMap
  onCommand: (def: RibbonCommandDef) => void
}) {
  const { enabled, pressed } = deriveButtonState(states, def.command)
  const glyph = GLYPH_ICON[def.id]
  const SvgIcon = SVG_ICON[def.id]
  return (
    <button
      type="button"
      disabled={!enabled}
      aria-label={def.label}
      aria-pressed={pressed}
      data-testid={`ribbon-${def.id}`}
      title={def.shortcut ? `${def.label} (${def.shortcut})` : def.label}
      onClick={() => onCommand(def)}
      className={`grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md transition-colors disabled:opacity-40 ${
        pressed ? 'bg-amber-bg text-ink shadow-[inset_0_-2px_0_var(--color-amber)]' : 'text-ink hover:bg-paper-3'
      }`}
    >
      {glyph ? <span className="font-sans text-[14px]" style={GLYPH_STYLE[def.id]}>{glyph}</span> : SvgIcon ? <SvgIcon size={16} /> : null}
    </button>
  )
}

function Divider() {
  return <div className="h-[22px] w-px shrink-0 bg-line" />
}

export function Ribbon({ app, activeTab, onTabChange, states, onCommand, onInsertLink, onInsertImage, impress, charFormatting }: RibbonProps) {
  const tabs = app === 'writer' ? WRITER_TABS : app === 'impress' ? IMPRESS_TABS : GENERIC_TABS
  const showHomeControls = activeTab === 'Home'

  return (
    <div className="flex h-12 shrink-0 items-center gap-3.5 border-b border-line bg-paper-2 px-4" data-testid="office-ribbon">
      <div className="flex h-full shrink-0 items-center gap-0.5">
        {tabs.map((tab) => (
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
            {activeTab === tab && (
              <span className="absolute inset-x-[9px] bottom-0 h-[2px] rounded-t-sm bg-amber" />
            )}
          </button>
        ))}
      </div>
      <Divider />
      {app !== 'impress' && showHomeControls && (
        <div className="flex min-w-0 flex-1 items-center gap-2.5 overflow-x-auto">
          <div className="flex shrink-0 items-center gap-0.5">
            <RibbonButton def={WRITER_HOME_COMMANDS[0]} states={states} onCommand={onCommand} />
            <RibbonButton def={WRITER_HOME_COMMANDS[1]} states={states} onCommand={onCommand} />
          </div>
          {app === 'writer' && (
            <>
              <Divider />
              <div className="flex h-[30px] shrink-0 min-w-[96px] items-center justify-between gap-1.5 rounded-md border border-line bg-paper px-2 text-[12.5px] text-ink">
                <span>Normal text</span>
                <IconChevronDown size={11} className="text-ink-3" />
              </div>
              {charFormatting && (
                <>
                  <Divider />
                  <FontPickers
                    fontName={charFormatting.state.fontName}
                    fontHeight={charFormatting.state.fontHeight}
                    onFontName={charFormatting.onFontName}
                    onFontHeight={charFormatting.onFontHeight}
                  />
                </>
              )}
            </>
          )}
          <Divider />
          <div className="flex shrink-0 items-center gap-0.5">
            {WRITER_HOME_COMMANDS.filter((d) => ['bold', 'italic', 'underline', 'strikethrough'].includes(d.id)).map(
              (def) => (
                <RibbonButton key={def.id} def={def} states={states} onCommand={onCommand} />
              ),
            )}
          </div>
          {app === 'writer' && (
            <>
              {charFormatting && (
                <>
                  <Divider />
                  <ColorPickers
                    textColor={charFormatting.state.textColor}
                    highlightColor={charFormatting.state.highlightColor}
                    onTextColor={charFormatting.onTextColor}
                    onHighlightColor={charFormatting.onHighlightColor}
                  />
                </>
              )}
              <Divider />
              <div className="flex shrink-0 items-center gap-0.5">
                {WRITER_HOME_COMMANDS.filter((d) => d.id === 'bullet-list' || d.id === 'numbered-list').map((def) => (
                  <RibbonButton key={def.id} def={def} states={states} onCommand={onCommand} />
                ))}
              </div>
              <Divider />
              <div className="flex shrink-0 items-center gap-0.5">
                {WRITER_HOME_COMMANDS.filter((d) => d.id.startsWith('align-')).map((def) => (
                  <RibbonButton key={def.id} def={def} states={states} onCommand={onCommand} />
                ))}
              </div>
              <Divider />
              <div className="flex shrink-0 items-center gap-0.5">
                {WRITER_HOME_COMMANDS.filter((d) => d.id.startsWith('indent-')).map((def) => (
                  <RibbonButton key={def.id} def={def} states={states} onCommand={onCommand} />
                ))}
              </div>
              <Divider />
              <div className="flex shrink-0 items-center gap-0.5">
                <button
                  type="button"
                  aria-label="Insert link"
                  data-testid="ribbon-link"
                  onClick={onInsertLink}
                  className="grid h-[30px] w-[30px] place-items-center rounded-md text-ink hover:bg-paper-3"
                >
                  <Icon name="link" size={16} />
                </button>
                <button
                  type="button"
                  aria-label="Insert image"
                  data-testid="ribbon-image"
                  onClick={onInsertImage}
                  className="grid h-[30px] w-[30px] place-items-center rounded-md text-ink hover:bg-paper-3"
                >
                  <Icon name="image" size={16} />
                </button>
                <button
                  type="button"
                  aria-label="Insert table"
                  data-testid="ribbon-table"
                  onClick={() => onCommand({ id: 'insert-table', label: 'Insert table', command: '.uno:InsertTable', group: 'Insert' })}
                  className="grid h-[30px] w-[30px] place-items-center rounded-md text-ink hover:bg-paper-3"
                >
                  <IconTable size={16} />
                </button>
              </div>
            </>
          )}
        </div>
      )}
      {app === 'impress' && (
        <ImpressRibbonContent
          activeTab={activeTab as ImpressTab}
          states={{ ...states, ...(impress?.states ?? {}) }}
          busy={impress?.busy ?? false}
          slideCount={impress?.slideCount ?? 0}
          onCommand={onCommand}
          onSlideOp={(op) => impress?.onSlideOp(op)}
          onApplyLayout={(layout) => impress?.onApplyLayout(layout)}
          onInsertImage={onInsertImage}
          onPresent={() => impress?.onPresent()}
        />
      )}
      {app !== 'impress' && !showHomeControls && <div className="flex-1" />}
    </div>
  )
}

export { WRITER_PARAGRAPH_STYLES }
