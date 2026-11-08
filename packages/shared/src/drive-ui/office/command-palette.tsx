/**
 * CommandPalette (task 1567) — the ⌘K "search commands, styles, files…"
 * overlay. `filterPaletteEntries` (../../lib/office/ribbon-commands) is the
 * pure, unit-tested filter; this component is the DOM shell + keyboard nav.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import { filterPaletteEntries, type PaletteEntry } from '../lib/office/ribbon-commands'

export interface CommandPaletteProps {
  open: boolean
  entries: PaletteEntry[]
  onClose: () => void
  onRun: (entry: PaletteEntry) => void
}

function highlight(label: string, query: string) {
  if (!query) return label
  const idx = label.toLowerCase().indexOf(query.toLowerCase())
  if (idx === -1) return label
  return (
    <>
      {label.slice(0, idx)}
      <mark className="bg-transparent font-bold text-amber-deep">{label.slice(idx, idx + query.length)}</mark>
      {label.slice(idx + query.length)}
    </>
  )
}

export function CommandPalette({ open, entries, onClose, onRun }: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const filtered = useMemo(() => filterPaletteEntries(entries, query), [entries, query])

  useEffect(() => {
    if (open) {
      setQuery('')
      setActiveIndex(0)
      // Focus after mount so the caret is visible immediately.
      setTimeout(() => inputRef.current?.focus(), 0)
    }
  }, [open])

  useEffect(() => {
    setActiveIndex(0)
  }, [query])

  if (!open) return null

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActiveIndex((i) => Math.min(i + 1, filtered.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActiveIndex((i) => Math.max(i - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const entry = filtered[activeIndex]
      if (entry) onRun(entry)
    }
  }

  let lastGroup: string | null = null

  return (
    <div className="absolute inset-0 z-[4] bg-black/20 backdrop-blur-[2px] dark:bg-black/48" onClick={onClose} data-testid="office-command-palette-scrim">
      <div
        role="dialog"
        aria-label="Command palette"
        onClick={(e) => e.stopPropagation()}
        className="absolute left-1/2 top-[12%] w-[560px] max-w-[90vw] -translate-x-1/2 overflow-hidden rounded-xl border border-line bg-paper-2 shadow-3"
        data-testid="office-command-palette"
      >
        <div className="flex items-center gap-2.5 border-b border-line px-4 py-3.5">
          <span className="font-mono text-[13px] font-semibold text-amber-deep">&gt;</span>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Search commands, styles, files…"
            aria-label="Search commands"
            data-testid="office-command-palette-input"
            className="flex-1 bg-transparent text-[15px] text-ink outline-none placeholder:text-ink-4"
          />
          <kbd className="rounded border border-line bg-paper-3 px-1.5 py-0.5 font-mono text-[11px] text-ink-4">esc</kbd>
        </div>
        <div className="max-h-[360px] overflow-auto py-1">
          {filtered.length === 0 && <div className="px-4 py-6 text-center text-[13px] text-ink-3">No matching commands</div>}
          {filtered.map((entry, i) => {
            const showGroup = entry.group !== lastGroup
            lastGroup = entry.group
            return (
              <div key={entry.id}>
                {showGroup && (
                  <div className="px-4 pb-1 pt-2.5 text-[10.5px] font-semibold uppercase tracking-wider text-ink-4">
                    {entry.group}
                  </div>
                )}
                <button
                  type="button"
                  data-testid={`palette-item-${entry.id}`}
                  onClick={() => onRun(entry)}
                  onMouseEnter={() => setActiveIndex(i)}
                  className={`flex w-full items-center gap-2.5 px-4 py-2 text-left text-[13.5px] ${
                    i === activeIndex ? 'bg-amber-bg' : 'text-ink'
                  }`}
                >
                  <span className="flex-1">{highlight(entry.label, query)}</span>
                  {entry.shortcut && <span className="font-mono text-[12px] text-ink-3">{entry.shortcut}</span>}
                </button>
              </div>
            )
          })}
        </div>
        <div className="flex gap-4 border-t border-line px-4 py-2.5 text-[11.5px] text-ink-3">
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-line px-1 font-mono text-[10.5px]">↑↓</kbd> to move
          </span>
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-line px-1 font-mono text-[10.5px]">↵</kbd> to run
          </span>
          <span>Runs entirely on this device</span>
        </div>
      </div>
    </div>
  )
}
