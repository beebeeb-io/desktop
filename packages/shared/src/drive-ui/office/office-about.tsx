/**
 * OfficeAbout (task 1585; the task 1567 open item) — the editor's in-app
 * license notice. The engine is an Executable Form of LibreOffice (MPL-2.0),
 * linked with Qt (LGPL-3.0) and shipping OFL fonts; MPL §3.2(a) requires
 * telling recipients how to get the Source Code Form. The full notice ships
 * next to the engine as `/office/<version>/THIRD_PARTY_NOTICES.txt`; this is
 * the visible way to reach it, and it names the source repository directly.
 *
 * A small status-bar button + popover rather than a new header control: it
 * is reference material, not a working tool (see DEVIATIONS.md).
 */

import { useEffect, useRef, useState } from 'react'
import { OFFICE_SOURCE_URL } from './office-engine-host'

export interface OfficeAboutProps {
  /** `/office/<version>/THIRD_PARTY_NOTICES.txt`, or null before the manifest is known. */
  noticesUrl: string | null
}

export function OfficeAbout({ noticesUrl }: OfficeAboutProps) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLSpanElement>(null)

  useEffect(() => {
    if (!open) return
    function onPointerDown(e: PointerEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false)
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const sourceLabel = OFFICE_SOURCE_URL.replace(/^https:\/\//, '')

  return (
    <span ref={rootRef} className="relative flex items-center">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="dialog"
        data-testid="office-about-button"
        className="font-sans text-[11px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
      >
        Licenses
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="About this editor"
          data-testid="office-about-panel"
          className="absolute bottom-[calc(100%+8px)] right-0 z-40 w-[min(320px,calc(100vw-32px))] rounded-lg border border-line bg-paper p-3.5 font-sans text-[12px] leading-relaxed text-ink-2 shadow-2"
        >
          <p className="mb-2 text-ink">
            This editor is LibreOffice, compiled to run entirely on this device. Beebeeb&apos;s changes and the build recipe
            are open source.
          </p>
          <p className="mb-3">
            Source:{' '}
            <a
              href={OFFICE_SOURCE_URL}
              target="_blank"
              rel="noopener noreferrer"
              data-testid="office-about-source"
              className="font-mono text-[11px] text-ink underline underline-offset-2"
            >
              {sourceLabel}
            </a>
          </p>
          {noticesUrl ? (
            <a
              href={noticesUrl}
              target="_blank"
              rel="noopener noreferrer"
              data-testid="office-licenses-link"
              className="text-ink underline underline-offset-2"
            >
              Third-party licenses (LibreOffice MPL-2.0, Qt LGPL-3.0, fonts OFL-1.1)
            </a>
          ) : (
            <span className="text-ink-4">The license notices load with the editor.</span>
          )}
        </div>
      )}
    </span>
  )
}
