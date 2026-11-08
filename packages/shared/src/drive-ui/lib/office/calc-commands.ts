/**
 * Calc ribbon + palette command tables (task 1567, Calc lane).
 *
 * Pure data + pure functions only -- no DOM, no bbOffice -- same isolation
 * contract as `./ribbon-commands.ts` (Writer's sibling table), which this
 * file deliberately does NOT import from or modify: the two apps' command
 * sets are genuinely different UNO commands (Calc's cell alignment is
 * `.uno:AlignLeft/HorizontalCenter/Right`, not Writer's paragraph-alignment
 * `.uno:LeftPara/CenterPara/RightPara`), and keeping them separate avoids any
 * shared-file collision with lanes editing ribbon-commands.ts concurrently.
 *
 * Every command string below was verified against the REAL rebuilt engine
 * artifact (repos/office/evidence/artifacts/emscripten, phase4-2026-09-27),
 * not assumed from LibreOffice documentation -- see the dated note this lane
 * appended to .claude/tasks/in-development/1567-*.md for the full probe
 * transcript. In particular:
 *   - Bold/Italic/Underline, AlignLeft/AlignHorizontalCenter/AlignRight,
 *     NumberFormatStandard/Currency/Percent/Decimal, DataFilterAutoFilter all
 *     report a real `{isEnabled, state: boolean}` toggle shape via onState,
 *     round-trip confirmed (dispatch flips `state` false->true).
 *   - AutoSum/SortAscending/SortDescending report `{isEnabled}` only (no
 *     `state` key) -- real action commands, never "pressed".
 *   - `.uno:JumpToTable` (sheet switching) and every insert-sheet command
 *     name tried (InsertTable / "Insert Sheet" / InsertSheet / Insert$Table)
 *     do NOT work in this build -- see calc-sheet-tabs.tsx's header comment.
 *     `.uno:RenameTable` DOES work (verified round-trip).
 */

export type UnoArgValue = string | number | boolean

export interface UnoArg {
  name: string
  value: UnoArgValue
}

// Deliberately the SAME allowed set `../../components/office/command-palette.tsx`
// (shared) already renders as a section header for `PaletteEntry.group` --
// keeping this union in lock-step (rather than a Calc-only "Data" tag) is
// what lets CALC_PALETTE_ENTRIES pass through that component's existing
// `entries: PaletteEntry[]` prop type unchanged, with zero edits to that
// shared file.
export type CalcCommandGroup = 'Format' | 'Insert' | 'Document' | 'Edit'

/** One ribbon button/dropdown that drives (and reflects) a `.uno:` command. */
export interface CalcCommandDef {
  /** Stable id for React keys / test selectors -- NOT the uno command itself. */
  id: string
  label: string
  /** The `.uno:` command dispatched on click. */
  command: string
  args?: UnoArg[]
  /** Keyboard shortcut shown in the palette (display only; not bound here). */
  shortcut?: string
  group: CalcCommandGroup
  /** True for a plain action (Undo, AutoSum, Sort...) that never has a
   *  "pressed" state -- matches the real engine's own reporting shape
   *  (verified: these commands' onState carries no `state` key at all). */
  action?: boolean
}

export interface UnoCommandState {
  isEnabled: boolean
  state: unknown
}

export type UnoStateMap = Record<string, UnoCommandState>

export interface CalcButtonState {
  enabled: boolean
  pressed: boolean
}

/** Commands verified (against the real engine) to report a boolean toggle
 *  `state` -- everything else in CALC_HOME_COMMANDS is a plain action. */
const TOGGLE_COMMANDS = new Set([
  '.uno:Bold',
  '.uno:Italic',
  '.uno:Underline',
  '.uno:AlignLeft',
  '.uno:AlignHorizontalCenter',
  '.uno:AlignRight',
  '.uno:NumberFormatCurrency',
  '.uno:NumberFormatPercent',
  '.uno:NumberFormatStandard',
  '.uno:DataFilterAutoFilter',
])

/**
 * Reads the current render state for one command out of the state map the
 * editor accumulates from `bbOffice.onState()` subscriptions. Before the
 * first callback has fired (subscription still resolving), a command
 * defaults to enabled+unpressed so the ribbon never flashes every button
 * greyed out on open -- same convention as ribbon-commands.ts's
 * `deriveButtonState`.
 */
export function deriveCalcButtonState(states: UnoStateMap, command: string): CalcButtonState {
  const entry = states[command]
  if (!entry) return { enabled: true, pressed: false }
  const pressed = TOGGLE_COMMANDS.has(command) && entry.state === true
  return { enabled: entry.isEnabled, pressed }
}

/** Calc's Home-tab ribbon, left to right, matching
 *  design/office-editor-shots/calc-{light,dark}.png. Font name/size and the
 *  fill-color swatch are decorative-only (no live binding), same treatment
 *  the Writer ribbon already gives its own "Normal text" style pill --
 *  established precedent, not a new deviation. The Table icon is rendered
 *  disabled: `.uno:InsertTable` has no dispatch handler in this engine build
 *  (verified), so it is honestly inert rather than silently wired to a
 *  command that does nothing. */
export const CALC_HOME_COMMANDS: CalcCommandDef[] = [
  { id: 'undo', label: 'Undo', command: '.uno:Undo', shortcut: '⌘Z', group: 'Edit', action: true },
  { id: 'redo', label: 'Redo', command: '.uno:Redo', shortcut: '⌘⇧Z', group: 'Edit', action: true },
  { id: 'bold', label: 'Bold', command: '.uno:Bold', shortcut: '⌘B', group: 'Format' },
  { id: 'italic', label: 'Italic', command: '.uno:Italic', shortcut: '⌘I', group: 'Format' },
  { id: 'underline', label: 'Underline', command: '.uno:Underline', shortcut: '⌘U', group: 'Format' },
  { id: 'align-left', label: 'Align left', command: '.uno:AlignLeft', group: 'Format' },
  { id: 'align-center', label: 'Align center', command: '.uno:AlignHorizontalCenter', group: 'Format' },
  { id: 'align-right', label: 'Align right', command: '.uno:AlignRight', group: 'Format' },
  { id: 'currency', label: 'Currency format', command: '.uno:NumberFormatCurrency', group: 'Format' },
  { id: 'inc-decimals', label: 'Add decimal place', command: '.uno:NumberFormatIncDecimals', group: 'Format', action: true },
  // "Data"-tab commands (mockup's Home-row Σ/sort/filter group) are tagged
  // 'Document' for the ⌘K palette's section header -- the closest bucket in
  // the shared PaletteEntry union (see CalcCommandGroup's own comment above).
  { id: 'autosum', label: 'Sum', command: '.uno:AutoSum', shortcut: '⌥⌘=', group: 'Document', action: true },
  { id: 'sort', label: 'Sort ascending', command: '.uno:SortAscending', group: 'Document', action: true },
  { id: 'filter', label: 'AutoFilter', command: '.uno:DataFilterAutoFilter', group: 'Document' },
]

/** Extra number formats offered from the palette only (not on the ribbon
 *  face, which shows Currency as its one real format button -- matching the
 *  mockup). */
export const CALC_FORMAT_PALETTE_COMMANDS: CalcCommandDef[] = [
  { id: 'format-standard', label: 'Number format: Standard', command: '.uno:NumberFormatStandard', group: 'Format' },
  { id: 'format-percent', label: 'Number format: Percent', command: '.uno:NumberFormatPercent', group: 'Format' },
]

/** Commands this editor tracks live via `bbOffice.onState()` -- passed as
 *  `office-editor.tsx`'s `commandsToTrack` for a Calc document. */
export const CALC_STATE_COMMANDS: string[] = CALC_HOME_COMMANDS.filter((d) => !d.action).map((d) => d.command)

export interface CalcPaletteEntry {
  id: string
  label: string
  group: CalcCommandGroup
  command: string
  args?: UnoArg[]
  shortcut?: string
}

function toPaletteEntry(def: CalcCommandDef): CalcPaletteEntry {
  return { id: def.id, label: def.label, group: def.group, command: def.command, args: def.args, shortcut: def.shortcut }
}

/** Full ⌘K palette table for a Calc document -- ribbon commands (excluding
 *  the disabled/decorative ones) + the extra number formats. */
export const CALC_PALETTE_ENTRIES: CalcPaletteEntry[] = [
  ...CALC_HOME_COMMANDS.map(toPaletteEntry),
  ...CALC_FORMAT_PALETTE_COMMANDS.map(toPaletteEntry),
]
