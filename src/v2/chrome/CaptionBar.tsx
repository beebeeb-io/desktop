// The Windows 11 title bar, drawn by the app (board: WinCaptionBar): the b and the title at the left, the
// caption buttons at the right, 46 x 32 each with 10 px glyphs. The bar is a drag region; double-click
// maximizes (Windows' own rule). Close hides the window: Beebeeb keeps running in the notification area
// (DW-1201 Rules). Windows' own window keys (Win+Up, Win+Down) stay Windows'.

import '../fonts/fonts.css'
import { useCallback, useEffect, useRef, type ReactNode } from 'react'
import { BBProductIcon } from '@beebeeb/shared/design'
import { hasTauri } from '../source'
import { GlobalBanner } from './Banners'
import { CommandsDialogHost } from './CommandsDialog'
import { ToastHost } from './Toast'

type WindowAction = 'minimize' | 'toggleMaximize' | 'hide'

async function windowAction(action: WindowAction): Promise<void> {
  if (!hasTauri()) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const w = getCurrentWindow()
  if (action === 'minimize') await w.minimize()
  else if (action === 'toggleMaximize') await w.toggleMaximize()
  else await w.hide()
}

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" aria-hidden="true">
      {children}
    </svg>
  )
}

function CaptionButton({ label, onClick, danger, children, buttonRef }: { label: string; onClick: () => void; danger?: boolean; children: ReactNode; buttonRef?: React.Ref<HTMLButtonElement> }) {
  return (
    <button
      ref={buttonRef}
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`grid h-8 w-[46px] place-items-center text-ink ${danger ? 'hover:bg-red hover:text-white' : 'hover:bg-paper-3'}`}
    >
      {children}
    </button>
  )
}

/** Snap layouts (spec 8.1): tell Rust where the maximize button is, in physical pixels of the client area, so the
 *  window can answer WM_NCHITTEST with HTMAXBUTTON there. No-op outside Tauri and off Windows. */
function useReportMaximizeRect(ref: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = ref.current
    if (!el || !hasTauri()) return
    let cancelled = false
    const report = async () => {
      const r = el.getBoundingClientRect()
      const k = window.devicePixelRatio || 1
      const { invoke } = await import('@tauri-apps/api/core')
      if (cancelled) return
      invoke('v2_set_maximize_rect', { x: Math.round(r.left * k), y: Math.round(r.top * k), width: Math.round(r.width * k), height: Math.round(r.height * k) }).catch(() => {})
    }
    void report()
    const observer = new ResizeObserver(() => { void report() })
    observer.observe(document.documentElement)
    window.addEventListener('resize', report)
    return () => { cancelled = true; observer.disconnect(); window.removeEventListener('resize', report) }
  }, [ref])
}

/** The bar alone (frames draw it without the live hosts). */
export function CaptionBarView({ title, maximizeRef }: { title: string; maximizeRef?: React.Ref<HTMLButtonElement> }) {
  const onDoubleClick = useCallback(() => { void windowAction('toggleMaximize') }, [])
  return (
    <header
      data-tauri-drag-region
      onDoubleClick={onDoubleClick}
      className="flex h-8 flex-none select-none items-center gap-[9px] border-b border-line bg-paper-2 pl-3 text-[12px] leading-4 text-ink-2"
    >
      <span data-tauri-drag-region className="inline-flex flex-none">
        <BBProductIcon product="drive" size={16} />
      </span>
      <span data-tauri-drag-region className="min-w-0 flex-1 truncate">{title}</span>
      <span className="flex flex-none self-stretch" onDoubleClick={(e) => e.stopPropagation()}>
        <CaptionButton label="Minimize" onClick={() => void windowAction('minimize')}>
          <Glyph><path d="M0 5.5h10" /></Glyph>
        </CaptionButton>
        <CaptionButton label="Maximize" buttonRef={maximizeRef} onClick={() => void windowAction('toggleMaximize')}>
          <Glyph><rect x=".5" y=".5" width="9" height="9" rx="1.5" /></Glyph>
        </CaptionButton>
        <CaptionButton label="Close" danger onClick={() => void windowAction('hide')}>
          <Glyph><path d="M.5.5l9 9M9.5.5l-9 9" /></Glyph>
        </CaptionButton>
      </span>
    </header>
  )
}

export function CaptionBar({ title }: { title: string }) {
  const maximizeRef = useRef<HTMLButtonElement>(null)
  useReportMaximizeRect(maximizeRef)
  return (
    <>
    <CaptionBarView title={title} maximizeRef={maximizeRef} />
    {/* The window-wide elements B03 owns, mounted with the bar until the shell mounts them (PR body). */}
    <GlobalBanner />
    <ToastHost />
    <CommandsDialogHost />
    </>
  )
}
