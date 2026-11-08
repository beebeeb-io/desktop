/**
 * ImpressRibbon (task 1567, Impress lane) — the Home/Insert/Design/
 * Transitions tab content rendered inside the shared `Ribbon` shell (see
 * ribbon.tsx's `app === 'impress'` branch, the one "registration hook" this
 * lane touches in that shared file).
 *
 * Home: Undo/Redo/Bold/Italic/Underline/Align-center + New/Duplicate/Delete
 * slide — every command real and probe-verified (impress-commands.ts).
 * Insert: image insert, wired to the real `bbOffice.insertImage()` call.
 * That call throws for any non-Writer document in the current engine build
 * (impress-commands.ts's header documents the exact root cause) — this tab
 * still fires the real call and reports the real failure via
 * `onInsertImageError`, rather than hiding the button or pretending it
 * works.
 * Design: the autolayout picker, dispatching `.uno:AssignLayout`.
 * Transitions: present as a real, switchable tab (matching Writer's own
 * Layout/Review tabs, which are equally switchable-but-contentless in this
 * codebase today) — no bridge call exists to read or set a slide's
 * transition properties (`impress-commands.ts`'s header), so this tab
 * deliberately shows no fake controls rather than buttons that would
 * silently do nothing.
 */

import {
  IMPRESS_HOME_COMMANDS,
  IMPRESS_LAYOUTS,
  IMPRESS_SLIDE_COMMANDS,
  PRESENT_COMMAND,
  layoutDispatchArgs,
  type LayoutDef,
} from '../lib/office/impress-commands'
import { deriveButtonState, type RibbonCommandDef, type UnoStateMap } from '../lib/office/ribbon-commands'
import { IconUndo, IconRedo } from './office-icons'
import { Icon } from '../../design'

export type ImpressTab = 'Home' | 'Insert' | 'Design' | 'Transitions'
export const IMPRESS_TABS: ImpressTab[] = ['Home', 'Insert', 'Design', 'Transitions']

const GLYPH_ICON: Record<string, string> = { bold: 'B', italic: 'I', underline: 'U' }
const GLYPH_STYLE: Record<string, React.CSSProperties> = {
  bold: { fontWeight: 700 },
  italic: { fontStyle: 'italic' },
  underline: { textDecoration: 'underline' },
}

function CmdButton({
  def,
  states,
  onCommand,
  testId,
}: {
  def: RibbonCommandDef
  states: UnoStateMap
  onCommand: (def: RibbonCommandDef) => void
  testId?: string
}) {
  const { enabled, pressed } = deriveButtonState(states, def.command)
  const glyph = GLYPH_ICON[def.id]
  return (
    <button
      type="button"
      disabled={!enabled}
      aria-label={def.label}
      aria-pressed={pressed}
      data-testid={testId ?? `impress-ribbon-${def.id}`}
      title={def.shortcut ? `${def.label} (${def.shortcut})` : def.label}
      onClick={() => onCommand(def)}
      className={`grid h-[30px] w-[30px] shrink-0 place-items-center rounded-md transition-colors disabled:opacity-40 ${
        pressed ? 'bg-amber-bg text-ink shadow-[inset_0_-2px_0_var(--color-amber)]' : 'text-ink hover:bg-paper-3'
      }`}
    >
      {glyph ? (
        <span className="font-sans text-[14px]" style={GLYPH_STYLE[def.id]}>
          {glyph}
        </span>
      ) : def.id === 'undo' ? (
        <IconUndo size={15} />
      ) : def.id === 'redo' ? (
        <IconRedo size={15} />
      ) : def.id === 'align-center' ? (
        <AlignCenterGlyph />
      ) : null}
    </button>
  )
}

function AlignCenterGlyph() {
  return (
    <svg width={15} height={15} viewBox="0 0 16 16" aria-hidden="true">
      <path d="M2 4h12M4 8h8M3 12h10" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" fill="none" />
    </svg>
  )
}

function Divider() {
  return <div className="h-[22px] w-px shrink-0 bg-line" />
}

export interface ImpressRibbonProps {
  activeTab: ImpressTab
  states: UnoStateMap
  busy: boolean
  onCommand: (def: RibbonCommandDef) => void
  onSlideOp: (op: keyof typeof IMPRESS_SLIDE_COMMANDS) => void
  onApplyLayout: (layout: LayoutDef) => void
  onInsertImage: () => void
  onPresent: () => void
  slideCount: number
}

export function ImpressRibbonContent({ activeTab, states, busy, onCommand, onSlideOp, onApplyLayout, onInsertImage, onPresent, slideCount }: ImpressRibbonProps) {
  if (activeTab === 'Home') {
    return (
      <div className="flex min-w-0 flex-1 items-center gap-2.5 overflow-x-auto">
        <div className="flex shrink-0 items-center gap-0.5">
          <CmdButton def={IMPRESS_HOME_COMMANDS[0]} states={states} onCommand={onCommand} />
          <CmdButton def={IMPRESS_HOME_COMMANDS[1]} states={states} onCommand={onCommand} />
        </div>
        <Divider />
        <button
          type="button"
          data-testid="impress-ribbon-new-slide"
          disabled={busy}
          onClick={() => onSlideOp('insert')}
          className="flex h-[30px] shrink-0 items-center gap-1.5 rounded-md border border-line bg-paper px-2.5 text-[12.5px] font-medium text-ink hover:bg-paper-3 disabled:opacity-40"
        >
          <Icon name="plus" size={12} />
          New slide
        </button>
        <Divider />
        <div className="flex shrink-0 items-center gap-0.5">
          {IMPRESS_HOME_COMMANDS.filter((d) => ['bold', 'italic', 'underline'].includes(d.id)).map((def) => (
            <CmdButton key={def.id} def={def} states={states} onCommand={onCommand} />
          ))}
        </div>
        <Divider />
        <CmdButton def={IMPRESS_HOME_COMMANDS.find((d) => d.id === 'align-center')!} states={states} onCommand={onCommand} />
        <Divider />
        <button
          type="button"
          data-testid="impress-ribbon-present"
          onClick={onPresent}
          disabled={slideCount < 1}
          title={`${PRESENT_COMMAND.label} (${PRESENT_COMMAND.shortcut})`}
          className="ml-auto flex h-[30px] shrink-0 items-center gap-1.5 rounded-md bg-amber px-3 text-[12.5px] font-semibold text-ink disabled:opacity-40"
        >
          <Icon name="play" size={12} />
          Present
        </button>
      </div>
    )
  }

  if (activeTab === 'Insert') {
    return (
      <div className="flex min-w-0 flex-1 items-center gap-2.5 overflow-x-auto">
        <button
          type="button"
          aria-label="Insert image"
          data-testid="impress-ribbon-image"
          onClick={onInsertImage}
          className="grid h-[30px] w-[30px] place-items-center rounded-md text-ink hover:bg-paper-3"
        >
          <Icon name="image" size={16} />
        </button>
      </div>
    )
  }

  if (activeTab === 'Design') {
    return (
      <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto">
        {IMPRESS_LAYOUTS.map((layout) => (
          <button
            key={layout.id}
            type="button"
            data-testid={`impress-layout-${layout.id}`}
            disabled={busy}
            onClick={() => onApplyLayout(layout)}
            title={layout.label}
            className="flex h-[30px] shrink-0 items-center gap-1.5 rounded-md border border-line bg-paper px-2.5 text-[12px] text-ink hover:bg-paper-3 disabled:opacity-40"
          >
            <span className="grid h-3.5 w-4.5 shrink-0 place-items-center rounded-[2px] border border-line bg-paper-3" />
            {layout.label}
          </button>
        ))}
      </div>
    )
  }

  // Transitions — a real, switchable tab with no fake controls (see this
  // file's header comment: no bridge call exists yet to set a slide
  // transition, so nothing here would actually do anything if drawn).
  return (
    <div className="flex min-w-0 flex-1 items-center text-[12px] text-ink-4">
      Slide transitions aren&apos;t wired up yet — coming in a later update.
    </div>
  )
}

export function applyLayoutArgs(layout: LayoutDef) {
  return layoutDispatchArgs(layout)
}
