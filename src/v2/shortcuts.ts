// The window's keyboard shortcuts, from the board (DW-1232 to DW-1234 list them; the frames' Actions name them).
// No Ctrl+Alt combination anywhere: AltGr is Ctrl+Alt on European keyboards and types characters (DW-1232 Rules).
// A section reacts by listening for the `bb:command` event with the command id in `detail`.

import { useEffect } from 'react'

export type CommandId =
  | 'settings'
  | 'find'
  | 'commands'
  | 'new-file'
  | 'new-folder'
  | 'upload'
  | 'select-all'
  | 'duplicate'
  | 'rename'
  | 'details'
  | 'toggle-details-panel'
  | 'trash'
  | 'undo'
  | 'redo'
  | 'save'
  | 'edit'
  | 'zoom-in'
  | 'zoom-out'
  | 'find-next'
  | 'present'
  | 'full-screen'
  | 'back'
  | 'forward'

export interface Shortcut {
  command: CommandId
  /** As the dialog shows it. */
  label: string
  /** The board frame that names it. */
  frame: string
  matches: (e: KeyboardEvent) => boolean
}

const ctrl = (key: string, shift = false) => (e: KeyboardEvent) =>
  e.ctrlKey && !e.altKey && e.shiftKey === shift && e.key.toLowerCase() === key
const plain = (key: string) => (e: KeyboardEvent) => !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === key

export const SHORTCUTS: Shortcut[] = [
  { command: 'settings', label: 'Ctrl+,', frame: 'DW-0401', matches: ctrl(',') },
  { command: 'find', label: 'Ctrl+F', frame: 'DW-0401', matches: ctrl('f') },
  { command: 'commands', label: 'Ctrl+K', frame: 'DW-1507', matches: ctrl('k') },
  { command: 'new-file', label: 'Ctrl+N', frame: 'DW-0414', matches: ctrl('n') },
  { command: 'new-folder', label: 'Ctrl+Shift+N', frame: 'DW-0414', matches: ctrl('n', true) },
  { command: 'upload', label: 'Ctrl+O', frame: 'DW-0414', matches: ctrl('o') },
  { command: 'select-all', label: 'Ctrl+A', frame: 'DW-0403', matches: ctrl('a') },
  { command: 'duplicate', label: 'Ctrl+D', frame: 'DW-0406', matches: ctrl('d') },
  { command: 'rename', label: 'F2', frame: 'DW-0410', matches: plain('F2') },
  { command: 'details', label: 'Alt+Enter', frame: 'DW-0411', matches: (e) => e.altKey && !e.ctrlKey && e.key === 'Enter' },
  { command: 'toggle-details-panel', label: 'Alt+Shift+P', frame: 'DW-0401', matches: (e) => e.altKey && e.shiftKey && !e.ctrlKey && e.key.toLowerCase() === 'p' },
  { command: 'trash', label: 'Delete', frame: 'DW-0405', matches: plain('Delete') },
  { command: 'undo', label: 'Ctrl+Z', frame: 'DW-0405', matches: ctrl('z') },
  { command: 'redo', label: 'Ctrl+Y', frame: 'DW-1232', matches: ctrl('y') },
  { command: 'save', label: 'Ctrl+S', frame: 'DW-1523', matches: ctrl('s') },
  { command: 'edit', label: 'Ctrl+E', frame: 'DW-1512', matches: ctrl('e') },
  { command: 'zoom-in', label: 'Ctrl+Plus', frame: 'DW-0603', matches: (e) => e.ctrlKey && !e.altKey && (e.key === '+' || e.key === '=') },
  { command: 'zoom-out', label: 'Ctrl+Minus', frame: 'DW-0603', matches: (e) => e.ctrlKey && !e.altKey && e.key === '-' },
  { command: 'find-next', label: 'F3', frame: 'DW-1232', matches: plain('F3') },
  { command: 'present', label: 'F5', frame: 'DW-1232', matches: plain('F5') },
  { command: 'full-screen', label: 'F11', frame: 'DW-1232', matches: plain('F11') },
  { command: 'back', label: 'Alt+Left', frame: 'DW-0401', matches: (e) => e.altKey && !e.ctrlKey && e.key === 'ArrowLeft' },
  { command: 'forward', label: 'Alt+Right', frame: 'DW-0401', matches: (e) => e.altKey && !e.ctrlKey && e.key === 'ArrowRight' },
]

export const COMMAND_EVENT = 'bb:command'

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el) return false
  return el.isContentEditable || el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT'
}

/** While typing in a field, only the commands a field does not own fire (Save, Find, Settings, Commands). */
const FIRES_WHILE_TYPING = new Set<CommandId>(['save', 'find', 'settings', 'commands', 'full-screen'])

export function useShortcuts(onCommand: (command: CommandId) => void): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const hit = SHORTCUTS.find((s) => s.matches(e))
      if (!hit) return
      if (isTyping(e.target) && !FIRES_WHILE_TYPING.has(hit.command)) return
      e.preventDefault()
      onCommand(hit.command)
      window.dispatchEvent(new CustomEvent<CommandId>(COMMAND_EVENT, { detail: hit.command }))
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCommand])
}

/** A section's hook for the commands it handles. */
export function useCommand(command: CommandId, handler: () => void): void {
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<CommandId>).detail === command) handler()
    }
    window.addEventListener(COMMAND_EVENT, on)
    return () => window.removeEventListener(COMMAND_EVENT, on)
  }, [command, handler])
}
