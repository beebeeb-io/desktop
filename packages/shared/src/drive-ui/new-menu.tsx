/**
 * "+ New" menu (task 1582) — Drive's single entry point for creating
 * something in the current folder: a folder, an Office document (when the build carries the
 * office editor), or a
 * text / Markdown file.
 *
 * Keyboard: the trigger opens with Enter/Space/ArrowDown (first item
 * focused) or ArrowUp (last item). Inside, ArrowUp/ArrowDown move, Home/End
 * jump, Escape and Tab close and return focus to the trigger. Items are real
 * buttons (Enter/Space activate). WAI-ARIA menu-button pattern.
 */

import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent } from 'react'
import { BBButton, Icon } from '../design'
import { visibleNewDocumentTypes, type NewDocumentType } from './lib/new-document'

export interface NewMenuProps {
  /** FEATURE_OFFICE_EDITOR (the build flag) — computed by the caller. */
  officeAvailable: boolean
  onNewFolder: () => void
  onNewDocument: (type: NewDocumentType) => void
  disabled?: boolean
  disabledReason?: string
  variant?: 'amber' | 'ghost' | 'default'
  size?: 'sm' | 'md' | 'lg'
  /** Where the popup anchors relative to the trigger. */
  align?: 'left' | 'right' | 'center'
  /** Distinguishes two menus on one page (toolbar vs empty state) in tests. */
  testIdPrefix?: string
}

export function NewMenu({
  officeAvailable,
  onNewFolder,
  onNewDocument,
  disabled,
  disabledReason,
  variant = 'amber',
  size = 'sm',
  align = 'right',
  testIdPrefix = 'new-menu',
}: NewMenuProps) {
  const [open, setOpen] = useState(false)
  const [moreOpen, setMoreOpen] = useState(false)
  const wrapRef = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const focusOnOpen = useRef<'first' | 'last'>('first')
  const menuId = useId()

  const types = visibleNewDocumentTypes(officeAvailable)
  const office = types.filter((t) => t.group === 'office')
  const openDocument = types.filter((t) => t.group === 'opendocument')
  const text = types.filter((t) => t.group === 'text')

  const items = useCallback(
    () => Array.from(menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []),
    [],
  )

  const close = useCallback((returnFocus: boolean) => {
    setOpen(false)
    setMoreOpen(false)
    if (returnFocus) {
      wrapRef.current?.querySelector<HTMLElement>('[data-new-menu-trigger]')?.focus()
    }
  }, [])

  // Focus the first/last item once the menu has rendered.
  useEffect(() => {
    if (!open) return
    const list = items()
    const target = focusOnOpen.current === 'last' ? list[list.length - 1] : list[0]
    target?.focus()
  }, [open, items])

  // Outside click closes (without stealing focus back — the user clicked
  // somewhere else on purpose).
  useEffect(() => {
    if (!open) return
    function onPointerDown(e: PointerEvent) {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) close(false)
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  }, [open, close])

  function onTriggerKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      focusOnOpen.current = e.key === 'ArrowUp' ? 'last' : 'first'
      setOpen(true)
    }
  }

  function onMenuKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const list = items()
    const idx = list.indexOf(document.activeElement as HTMLElement)
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        list[(idx + 1) % list.length]?.focus()
        break
      case 'ArrowUp':
        e.preventDefault()
        list[(idx - 1 + list.length) % list.length]?.focus()
        break
      case 'Home':
        e.preventDefault()
        list[0]?.focus()
        break
      case 'End':
        e.preventDefault()
        list[list.length - 1]?.focus()
        break
      case 'Escape':
        e.preventDefault()
        e.stopPropagation()
        close(true)
        break
      case 'Tab':
        close(true)
        break
    }
  }

  function pick(type: NewDocumentType) {
    close(false)
    onNewDocument(type)
  }

  const itemClass =
    'w-full flex items-center gap-2.5 px-3 py-1.5 text-left text-[13px] text-ink-2 hover:bg-paper-2 hover:text-ink focus:bg-paper-2 focus:text-ink focus:outline-none transition-colors'

  function renderTypeItem(t: NewDocumentType) {
    // OpenDocument items sit under "More formats": indented to read as its
    // children.
    return (
      <button
        key={t.id}
        type="button"
        role="menuitem"
        data-testid={`${testIdPrefix}-${t.id}`}
        onClick={() => pick(t)}
        className={`${itemClass} ${t.group === 'opendocument' ? 'pl-8' : ''}`}
      >
        <Icon name={t.icon} size={14} className="shrink-0 text-ink-3" />
        <span className="flex-1">{t.label}</span>
        <span className="font-mono text-[11px] text-ink-4">.{t.ext}</span>
      </button>
    )
  }

  const alignClass = align === 'right' ? 'right-0' : align === 'left' ? 'left-0' : 'left-1/2 -translate-x-1/2'

  return (
    <div ref={wrapRef} className="relative">
      <BBButton
        size={size}
        variant={variant}
        data-new-menu-trigger
        data-testid={`${testIdPrefix}-trigger`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        title={disabled ? disabledReason : undefined}
        onClick={() => {
          focusOnOpen.current = 'first'
          if (open) close(false)
          else setOpen(true)
        }}
        onKeyDown={onTriggerKeyDown}
        className="gap-1.5"
      >
        <Icon name="plus" size={13} /> New
        <Icon name="chevron-down" size={11} className="opacity-70" />
      </BBButton>

      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label="Create new"
          data-testid={`${testIdPrefix}-list`}
          onKeyDown={onMenuKeyDown}
          className={`absolute ${alignClass} top-full mt-1 z-40 min-w-[240px] rounded-md border border-line-2 bg-paper py-1 shadow-2`}
        >
          <button
            type="button"
            role="menuitem"
            data-testid={`${testIdPrefix}-folder`}
            onClick={() => {
              close(false)
              onNewFolder()
            }}
            className={itemClass}
          >
            <Icon name="folder" size={14} className="shrink-0 text-ink-3" />
            <span className="flex-1">Folder</span>
          </button>

          {office.length > 0 && (
            <>
              <div role="separator" className="my-1 h-px bg-line" />
              {office.map(renderTypeItem)}
              {openDocument.length > 0 && (
                <>
                  <button
                    type="button"
                    role="menuitem"
                    aria-expanded={moreOpen}
                    data-testid={`${testIdPrefix}-more-formats`}
                    onClick={() => setMoreOpen((v) => !v)}
                    className={itemClass}
                  >
                    <Icon name={moreOpen ? 'chevron-down' : 'chevron-right'} size={12} className="shrink-0 text-ink-4" />
                    <span className="flex-1 text-ink-3">More formats</span>
                    <span className="text-[11px] text-ink-4">OpenDocument</span>
                  </button>
                  {moreOpen && openDocument.map(renderTypeItem)}
                </>
              )}
            </>
          )}

          <div role="separator" className="my-1 h-px bg-line" />
          {text.map(renderTypeItem)}
        </div>
      )}
    </div>
  )
}
