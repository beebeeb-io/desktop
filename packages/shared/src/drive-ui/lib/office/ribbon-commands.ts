/**
 * Ribbon + command-palette command tables for the office editor (task 1567).
 *
 * Pure data + pure functions only — no DOM, no bbOffice, no React. The
 * Ribbon/CommandPalette components read these tables and call
 * `deriveButtonState` against whatever `bbOffice.onState()` has reported so
 * far; this module never talks to the engine itself, so it is fully
 * unit-testable (mirrors `../editor-conflict.ts` / `../text-editability.ts`).
 */

export type UnoArgValue = string | number | boolean

export interface UnoArg {
  name: string
  value: UnoArgValue
}

/** One ribbon button/dropdown that drives (and reflects) a `.uno:` command. */
export interface RibbonCommandDef {
  /** Stable id for React keys / test selectors — NOT the uno command itself. */
  id: string
  label: string
  /** The `.uno:` command dispatched on click. */
  command: string
  args?: UnoArg[]
  /** Keyboard shortcut shown in the palette (display only; not bound here). */
  shortcut?: string
  /** Command-palette grouping. */
  group: 'Format' | 'Insert' | 'Document' | 'Edit'
}

/** Live state for one `.uno:` command, as reported by `bbOffice.onState()`. */
export interface UnoCommandState {
  isEnabled: boolean
  /** Boolean for toggle commands (Bold/Italic/…); other shapes for the rest. */
  state: unknown
}

export type UnoStateMap = Record<string, UnoCommandState>

/** Derived, render-ready state for one ribbon button. */
export interface RibbonButtonState {
  enabled: boolean
  /** True for a toggle command (Bold, bullet list, an alignment button, …)
   *  currently active. Always false for a non-toggle command (Undo, Insert
   *  Image, …) — those have no "pressed" concept. */
  pressed: boolean
}

/** Commands whose `state` payload is a plain boolean toggle (aria-pressed
 *  buttons in the Home tab). Everything else in `WRITER_HOME_COMMANDS` that
 *  isn't in this set is treated as a plain action button (never pressed). */
const TOGGLE_COMMANDS = new Set([
  '.uno:Bold',
  '.uno:Italic',
  '.uno:Underline',
  '.uno:Strikeout',
  '.uno:LeftPara',
  '.uno:CenterPara',
  '.uno:RightPara',
  '.uno:JustifyPara',
  '.uno:DefaultBullet',
  '.uno:DefaultNumbering',
])

/**
 * Reads the current render state for one command out of the state map the
 * editor accumulates from `bbOffice.onState()` subscriptions. Before the
 * first callback has fired for a command (subscription still resolving, or
 * this command was never subscribed at all — e.g. the editor for a
 * different app), a command defaults to enabled+unpressed rather than
 * disabled, so the ribbon never flashes every button greyed out on open.
 */
export function deriveButtonState(states: UnoStateMap, command: string): RibbonButtonState {
  const entry = states[command]
  if (!entry) return { enabled: true, pressed: false }
  const pressed = TOGGLE_COMMANDS.has(command) && entry.state === true
  return { enabled: entry.isEnabled, pressed }
}

/** Writer's Home-tab ribbon, left to right, matching design/office-editor.html. */
export const WRITER_HOME_COMMANDS: RibbonCommandDef[] = [
  { id: 'undo', label: 'Undo', command: '.uno:Undo', shortcut: '⌘Z', group: 'Edit' },
  { id: 'redo', label: 'Redo', command: '.uno:Redo', shortcut: '⌘⇧Z', group: 'Edit' },
  { id: 'bold', label: 'Bold', command: '.uno:Bold', shortcut: '⌘B', group: 'Format' },
  { id: 'italic', label: 'Italic', command: '.uno:Italic', shortcut: '⌘I', group: 'Format' },
  { id: 'underline', label: 'Underline', command: '.uno:Underline', shortcut: '⌘U', group: 'Format' },
  { id: 'strikethrough', label: 'Strikethrough', command: '.uno:Strikeout', group: 'Format' },
  { id: 'bullet-list', label: 'Bulleted list', command: '.uno:DefaultBullet', group: 'Format' },
  { id: 'numbered-list', label: 'Numbered list', command: '.uno:DefaultNumbering', group: 'Format' },
  { id: 'align-left', label: 'Align left', command: '.uno:LeftPara', shortcut: '⌘L', group: 'Format' },
  { id: 'align-center', label: 'Align center', command: '.uno:CenterPara', shortcut: '⌘E', group: 'Format' },
  { id: 'align-right', label: 'Align right', command: '.uno:RightPara', shortcut: '⌘R', group: 'Format' },
  { id: 'align-justify', label: 'Justify', command: '.uno:JustifyPara', shortcut: '⌘J', group: 'Format' },
  { id: 'indent-more', label: 'Increase indent', command: '.uno:IncrementIndent', group: 'Format' },
  { id: 'indent-less', label: 'Decrease indent', command: '.uno:DecrementIndent', group: 'Format' },
]

/** Writer's heading/paragraph-style quick set (Format menu + palette). */
export const WRITER_PARAGRAPH_STYLES: RibbonCommandDef[] = [
  {
    id: 'style-normal',
    label: 'Normal text',
    command: '.uno:StyleApply',
    args: [{ name: 'Style', value: 'Default Paragraph Style' }, { name: 'FamilyName', value: 'ParagraphStyles' }],
    group: 'Format',
  },
  {
    id: 'style-h1',
    label: 'Heading 1',
    command: '.uno:StyleApply',
    args: [{ name: 'Style', value: 'Heading 1' }, { name: 'FamilyName', value: 'ParagraphStyles' }],
    shortcut: '⌥⌘1',
    group: 'Format',
  },
  {
    id: 'style-h2',
    label: 'Heading 2',
    command: '.uno:StyleApply',
    args: [{ name: 'Style', value: 'Heading 2' }, { name: 'FamilyName', value: 'ParagraphStyles' }],
    shortcut: '⌥⌘2',
    group: 'Format',
  },
  {
    id: 'style-h3',
    label: 'Heading 3',
    command: '.uno:StyleApply',
    args: [{ name: 'Style', value: 'Heading 3' }, { name: 'FamilyName', value: 'ParagraphStyles' }],
    shortcut: '⌥⌘3',
    group: 'Format',
  },
]

/** Document-level commands offered from the palette (not on the ribbon face). */
export const WRITER_DOCUMENT_COMMANDS: RibbonCommandDef[] = [
  { id: 'insert-table', label: 'Insert table…', command: '.uno:InsertTable', group: 'Insert' },
  { id: 'page-layout', label: 'Page layout…', command: '.uno:PageDialog', group: 'Document' },
  { id: 'word-count', label: 'Word count…', command: '.uno:WordCountDialog', group: 'Document' },
]

/** One entry in the ⌘K command palette. */
export interface PaletteEntry {
  id: string
  label: string
  group: RibbonCommandDef['group']
  command: string
  args?: UnoArg[]
  shortcut?: string
}

function toPaletteEntry(def: RibbonCommandDef): PaletteEntry {
  return { id: def.id, label: def.label, group: def.group, command: def.command, args: def.args, shortcut: def.shortcut }
}

/** Full palette table for a Writer document — ribbon commands + styles +
 *  document actions, in this fixed priority order (Format first, matching
 *  the mockup's "heading" query surfacing Format above Insert/Document). */
export const WRITER_PALETTE_ENTRIES: PaletteEntry[] = [
  ...WRITER_PARAGRAPH_STYLES.map(toPaletteEntry),
  ...WRITER_HOME_COMMANDS.map(toPaletteEntry),
  ...WRITER_DOCUMENT_COMMANDS.map(toPaletteEntry),
]

/**
 * Case-insensitive substring match over an entry's label, preserving table
 * order among equal matches (a stable sort, not a fuzzy/scored one — the
 * mockup's own example expects "Heading 1/2/3" to stay in that order for a
 * "heading" query, not be re-ordered by some relevance heuristic).
 */
export function filterPaletteEntries(entries: PaletteEntry[], query: string): PaletteEntry[] {
  const q = query.trim().toLowerCase()
  if (!q) return entries
  return entries.filter((e) => e.label.toLowerCase().includes(q))
}
