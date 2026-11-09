// Small Windows-style building blocks for the Files frames: modal, menu, icon button. Neutral grey, Amber only on the primary action.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Icon, type IconName } from '@beebeeb/shared/design'
import { useFocusTrap } from '@beebeeb/shared/drive-ui'

export const FOCUS = 'focus-visible:outline-2 focus-visible:outline-ink-3'
export const BTN = `inline-flex h-8 flex-none whitespace-nowrap items-center justify-center rounded-md border border-line bg-paper px-4 text-[13px] font-medium text-ink hover:bg-paper-2 ${FOCUS}`
export const BTN_AMBER = `inline-flex h-8 items-center justify-center rounded-md bg-amber px-4 text-[13px] font-medium text-[oklch(0.22_0.01_70)] hover:brightness-95 ${FOCUS}`

export function CloseX({ onClick, label = 'Close (Esc)' }: { onClick?: () => void; label?: string }) {
  return (
    <button type="button" aria-label={label} title={label} onClick={onClick} className={`grid h-7 w-7 flex-none place-items-center rounded-md text-ink-3 hover:bg-paper-3 ${FOCUS}`}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
    </button>
  )
}

export function IconButton({ icon, label, onClick, pressed, disabled, flip }: { icon: IconName; label: string; onClick?: () => void; pressed?: boolean; disabled?: boolean; flip?: boolean }) {
  return (
    <button
      type="button" aria-label={label} title={label} aria-pressed={pressed} disabled={disabled} onClick={onClick}
      className={`grid h-8 w-8 flex-none place-items-center rounded-md ${pressed ? 'bg-paper-3 text-ink' : 'text-ink-2 hover:bg-paper-2'} disabled:opacity-40 ${FOCUS}`}
    >
      <span className={flip ? 'inline-flex rotate-180' : 'inline-flex'}><Icon name={icon} size={16} /></span>
    </button>
  )
}

/** A centred dialog over the window. Esc closes; focus stays inside. */
export function Modal({ title, onClose, width = 460, children, footer, label, placement = 'center' }: {
  title: ReactNode; onClose?: () => void; width?: number; children: ReactNode; footer?: ReactNode; label?: string; placement?: 'center' | 'top'
}) {
  const ref = useFocusTrap<HTMLDivElement>(true)
  useEffect(() => {
    const on = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose?.() }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [onClose])
  return (
    <div role="presentation" className={`fixed inset-0 z-50 flex justify-center bg-ink/25 ${placement === 'top' ? 'items-start pt-24' : 'items-center'}`} onMouseDown={(e) => { if (e.target === e.currentTarget) onClose?.() }}>
      <div ref={ref} role="dialog" aria-modal="true" aria-label={label ?? (typeof title === 'string' ? title : undefined)} data-dialog={label ?? (typeof title === 'string' ? title : '')} style={{ width }} className="max-w-[calc(100vw-48px)] rounded-xl border border-line bg-paper shadow-2xl">
        <div className="px-5 pb-1 pt-4 text-[15px] font-semibold leading-6 text-ink">{title}</div>
        <div className="px-5 pb-4 pt-1 text-[13px] leading-5 text-ink-2">{children}</div>
        {footer && <div className="flex items-center gap-2 border-t border-line px-5 py-3">{footer}</div>}
      </div>
    </div>
  )
}

export interface MenuEntry {
  id: string
  label: string
  shortcut?: string
  disabled?: boolean
  checked?: boolean
  dividerAfter?: boolean
  danger?: boolean
  /** A submenu opens to the right. */
  submenu?: MenuEntry[]
  /** Draw the submenu open (frames). */
  open?: boolean
  /** A short reason for a dimmed item. */
  hint?: string
}

function MenuPanel({ entries, onPick, label, forcedOpen }: { entries: MenuEntry[]; onPick: (id: string) => void; label: string; forcedOpen?: boolean }) {
  const ref = useRef<HTMLDivElement>(null)
  const [sub, setSub] = useState<string | null>(entries.find((e) => e.open)?.id ?? null)
  const buttons = () => Array.from(ref.current?.querySelectorAll<HTMLButtonElement>(':scope > button:not([disabled])') ?? [])
  useEffect(() => { if (!forcedOpen) buttons()[0]?.focus() }, [forcedOpen])
  const onKeyDown = (e: React.KeyboardEvent) => {
    const list = buttons()
    const i = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowDown') { e.preventDefault(); list[(i + 1) % list.length]?.focus() }
    else if (e.key === 'ArrowUp') { e.preventDefault(); list[(i - 1 + list.length) % list.length]?.focus() }
    else if (e.key.length === 1 && /\S/.test(e.key)) {
      const next = list.find((b) => b.textContent?.trim().toLowerCase().startsWith(e.key.toLowerCase()))
      next?.focus()
    }
  }
  return (
    <div ref={ref} role="menu" tabIndex={-1} aria-label={label} onKeyDown={onKeyDown} className="relative min-w-[264px] rounded-lg border border-line bg-paper py-1 text-[13px] shadow-xl">
      {entries.map((e) => (
        <div key={e.id} className="relative">
          <button
            type="button" role="menuitem" disabled={e.disabled} data-menu-item={e.id} title={e.disabled ? e.hint : undefined}
            aria-haspopup={e.submenu ? 'menu' : undefined}
            onMouseEnter={() => setSub(e.submenu ? e.id : null)}
            onClick={() => (e.submenu ? setSub(e.id) : onPick(e.id))}
            className={`flex h-8 w-full items-center gap-3 px-3 text-left hover:bg-paper-2 focus-visible:bg-paper-2 focus-visible:outline-none disabled:text-ink-3 disabled:opacity-60 disabled:hover:bg-transparent ${e.danger ? 'text-ink' : 'text-ink'}`}
          >
            <span className="grid w-4 flex-none place-items-center text-ink-2">{e.checked ? <Icon name="check" size={13} /> : null}</span>
            <span className="min-w-0 flex-1 truncate">{e.label}</span>
            {e.shortcut && <span className="font-mono text-[11px] text-ink-3">{e.shortcut}</span>}
            {e.submenu && <Icon name="chevron-right" size={12} />}
          </button>
          {e.submenu && (sub === e.id) && (
            <div className="absolute left-full top-0 z-10 ml-1"><MenuPanel entries={e.submenu} onPick={onPick} label={e.label} forcedOpen /></div>
          )}
          {e.dividerAfter && <div role="separator" className="my-1 h-px bg-line" />}
        </div>
      ))}
    </div>
  )
}

/** A menu at a fixed point, closed by Esc or a click outside. */
export function Menu({ x, y, entries, onPick, onClose, label }: { x: number; y: number; entries: MenuEntry[]; onPick: (id: string) => void; onClose?: () => void; label: string }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose?.() }
    const down = (e: MouseEvent) => { if (!(e.target as HTMLElement).closest('[role=menu]')) onClose?.() }
    window.addEventListener('keydown', key)
    window.addEventListener('mousedown', down)
    return () => { window.removeEventListener('keydown', key); window.removeEventListener('mousedown', down) }
  }, [onClose])
  const left = Math.max(8, Math.min(x, window.innerWidth - 280))
  const top = Math.max(8, Math.min(y, window.innerHeight - 40 - entries.length * 33))
  return (
    // A click inside the menu stays inside: the view behind closes menus on its own click, which would close a
    // submenu's parent (Sort by ▸) before it opens (M3a).
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div className="fixed z-50" style={{ left, top }} data-menu={label} onClick={(e) => e.stopPropagation()}>
      <MenuPanel entries={entries} onPick={(id) => { onPick(id); onClose?.() }} label={label} />
    </div>
  )
}
