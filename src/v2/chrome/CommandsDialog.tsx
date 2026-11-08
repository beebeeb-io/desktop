// "Commands and keyboard shortcuts" (DW-1232 to DW-1234): a dialog with one list per menu; Ctrl+/ opens it from
// anywhere in the window, the account menu's Keyboard shortcuts opens it too, Close or Esc closes it.

import { useEffect, useId, useRef, useState } from 'react'
import { EDITOR_COMMANDS, FILES_COMMANDS, NAVIGATION_COMMANDS, type CommandList } from './commands'

export type CommandsSet = 'files' | 'navigation' | 'editor'
export const OPEN_COMMANDS_EVENT = 'bb:open-commands'

const SETS: Record<CommandsSet, CommandList[]> = { files: FILES_COMMANDS, navigation: NAVIGATION_COMMANDS, editor: EDITOR_COMMANDS }

/** The account menu's Keyboard shortcuts calls this. `set` picks the lists; an open document shows the editor's. */
export function openCommandsDialog(set?: CommandsSet): void {
  window.dispatchEvent(new CustomEvent(OPEN_COMMANDS_EVENT, { detail: { set } }))
}

export interface CommandsTab { label: string; active: boolean; onSelect: () => void }

export function CommandsDialog({ lists, onClose, tabs }: { lists: CommandList[]; onClose: () => void; tabs?: CommandsTab[] }) {
  const titleId = useId()
  const closeRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    closeRef.current?.focus()
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose() } }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-ink/30 p-6" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div role="dialog" aria-modal="true" aria-labelledby={titleId} className="flex max-h-[calc(100vh-48px)] w-full max-w-[1120px] flex-col rounded-xl border border-line bg-paper shadow-2xl">
        <div className="flex-none px-6 pb-3 pt-5">
          <h2 id={titleId} className="m-0 text-[16px] font-semibold leading-6 text-ink">Commands and keyboard shortcuts</h2>
          <p className="m-0 mt-1 text-[13px] leading-5 text-ink-3">Ctrl+/ or the account menu. Each command lives in the window, a right-click menu or Settings; this list shows where.</p>
        </div>
        {tabs && (
          <div role="tablist" className="flex flex-none gap-1 px-6 pb-2">
            {tabs.map((t) => (
              <button key={t.label} type="button" role="tab" aria-selected={t.active} onClick={t.onSelect} className={`h-7 rounded-md px-3 text-[13px] ${t.active ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2 hover:bg-paper-2'}`}>{t.label}</button>
            ))}
          </div>
        )}
        <div className="grid min-h-0 flex-1 gap-x-8 overflow-y-auto px-6 pb-4" style={{ gridTemplateColumns: `repeat(${lists.length}, minmax(0, 1fr))` }}>
          {lists.map((list) => (
            <section key={list.title} aria-label={list.title}>
              <h3 className="sticky top-0 m-0 bg-paper py-2 text-[11px] font-semibold uppercase leading-4 tracking-wide text-ink-3">{list.title}</h3>
              <ul className="m-0 list-none p-0">
                {list.rows.map((row) => (
                  <li key={row.name} data-command className="flex items-baseline justify-between gap-3 py-1 text-[13px] leading-5">
                    <span className="text-ink">{row.name}</span>
                    <span className="text-right font-mono text-[12px] text-ink-3">{row.where}</span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <div className="flex flex-none justify-end border-t border-line px-6 py-3">
          <button ref={closeRef} type="button" onClick={onClose} className="h-8 rounded-md border border-line bg-paper px-4 text-[13px] font-medium text-ink hover:bg-paper-2 focus-visible:outline-2 focus-visible:outline-ink-3">Close</button>
        </div>
      </div>
    </div>
  )
}

/** Mount once: Ctrl+/ toggles it, and openCommandsDialog() opens it. */
export function CommandsDialogHost() {
  const [set, setSet] = useState<CommandsSet | null>(null)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key === '/') { e.preventDefault(); setSet((s) => (s ? null : 'files')) }
    }
    const onOpen = (e: Event) => setSet((e as CustomEvent<{ set?: CommandsSet }>).detail?.set ?? 'files')
    window.addEventListener('keydown', onKey)
    window.addEventListener(OPEN_COMMANDS_EVENT, onOpen)
    return () => { window.removeEventListener('keydown', onKey); window.removeEventListener(OPEN_COMMANDS_EVENT, onOpen) }
  }, [])
  if (!set) return null
  const tabs: CommandsTab[] | undefined = set === 'editor' ? undefined : [
    { label: 'Beebeeb, File, Edit', active: set === 'files', onSelect: () => setSet('files') },
    { label: 'View, Go, Window, Help', active: set === 'navigation', onSelect: () => setSet('navigation') },
  ]
  return <CommandsDialog lists={SETS[set]} tabs={tabs} onClose={() => setSet(null)} />
}
