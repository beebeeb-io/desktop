/**
 * ImpressPresentOverlay (task 1567, Impress lane) — "plays the deck full
 * screen inside the page (no external windows)" per the task brief.
 *
 * The engine itself only ever paints to the ONE `#qtcanvas` this whole app
 * shares (there is no second window this WASM build could ever open — see
 * office-engine-host.tsx's own header on why the engine lives in a single
 * same-origin iframe) — dispatching `.uno:Presentation` therefore already
 * satisfies "no external windows" architecturally. This component's job is
 * the REACT side of "full screen": hide Beebeeb's own chrome (header,
 * ribbon, filmstrip, status bar — all done by OfficeEditor wrapping them in
 * `{!presenting && …}`, not by this file) and best-effort request real
 * Fullscreen on the shared container, so the canvas the engine is already
 * driving fills the viewport.
 *
 * Exit does NOT depend on `.uno:Escape` succeeding — probed directly against
 * the real engine (impress-commands.ts's header): outside of an
 * actually-focused running slideshow, that dispatch has no handler at all
 * (`no dispatch handler for .uno:Escape`). It's still attempted, best-effort,
 * in case the running slideshow itself IS the focused target once
 * `.uno:Presentation` is live — but the RELIABLE exit is this component's own
 * "Exit presentation" pill / a real Escape keypress caught by the HOST page
 * (OfficeEditor's existing global keydown listener, extended one line for
 * this), which always restores our chrome regardless of what the engine did.
 */

import { useEffect, useRef, useState } from 'react'
import { Icon } from '../../design'

export interface ImpressPresentOverlayProps {
  onExit: () => void
}

export function ImpressPresentOverlay({ onExit }: ImpressPresentOverlayProps) {
  const [showControls, setShowControls] = useState(true)
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    function armHideTimer() {
      setShowControls(true)
      if (hideTimer.current) clearTimeout(hideTimer.current)
      hideTimer.current = setTimeout(() => setShowControls(false), 2500)
    }
    armHideTimer()
    window.addEventListener('mousemove', armHideTimer)
    return () => {
      window.removeEventListener('mousemove', armHideTimer)
      if (hideTimer.current) clearTimeout(hideTimer.current)
    }
  }, [])

  return (
    <div
      className={`pointer-events-none absolute right-4 top-4 z-[5] transition-opacity duration-300 ${showControls ? 'opacity-100' : 'opacity-0'}`}
      data-testid="impress-present-controls"
    >
      <button
        type="button"
        onClick={onExit}
        data-testid="impress-present-exit"
        className="pointer-events-auto flex h-9 items-center gap-1.5 rounded-full border border-line/60 bg-black/55 px-3.5 text-[12.5px] font-medium text-white backdrop-blur-sm hover:bg-black/70"
      >
        <Icon name="x" size={13} />
        Exit presentation
        <kbd className="ml-1 rounded border border-white/25 px-1 font-mono text-[10px] opacity-75">esc</kbd>
      </button>
    </div>
  )
}
