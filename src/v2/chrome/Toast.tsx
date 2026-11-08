// Toasts (DW-0107 to DW-0109): one message at a time at the bottom of the content area, never over the status
// line. A done message lasts 3 s, one with Undo 6 s, a failure until Try again or 8 s. Never a raw error code.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'

export type ToastKind = 'done' | 'undo' | 'failed'
export interface ToastMessage {
  kind: ToastKind
  text: string
  /** Called by Undo (and Ctrl+Z while the toast shows) or Try again. */
  onAction?: () => void
}

export const TOAST_MS: Record<ToastKind, number> = { done: 3000, undo: 6000, failed: 8000 }
export const TOAST_EVENT = 'bb:toast'

/** Anything may show a toast without a provider: `showToast({ kind: 'done', text: 'Link copied' })`. */
export function showToast(message: ToastMessage): void {
  window.dispatchEvent(new CustomEvent<ToastMessage>(TOAST_EVENT, { detail: message }))
}

const ToastContext = createContext<(m: ToastMessage) => void>(showToast)
export const useToast = () => useContext(ToastContext)

/** The message itself, for the host and for frames. */
export function ToastView({ message, onAction, onClose }: { message: ToastMessage; onAction?: () => void; onClose?: () => void }) {
  const label = message.kind === 'undo' ? 'Undo' : message.kind === 'failed' ? 'Try again' : null
  return (
    <div
      role={message.kind === 'failed' ? 'alert' : 'status'}
      data-toast={message.kind}
      className="pointer-events-auto flex min-h-10 items-center gap-3 rounded-lg border border-line bg-ink py-2 pl-4 pr-2 text-[13px] leading-5 text-paper shadow-lg"
    >
      <span>{message.text}</span>
      {label && (
        <button type="button" onClick={() => { onAction?.(); onClose?.() }} className="inline-flex h-7 items-center gap-2 rounded-md px-2 font-medium text-paper hover:bg-paper/15 focus-visible:outline-2 focus-visible:outline-paper">
          {label}
          {message.kind === 'undo' && <span className="font-mono text-[11px] opacity-70">Ctrl+Z</span>}
        </button>
      )}
      <span className="w-0.5" />
    </div>
  )
}

/** Mount once. Sits above the 28 px status line, centred in the content area. */
export function ToastHost({ children }: { children?: ReactNode }) {
  const [message, setMessage] = useState<ToastMessage | null>(null)
  const timer = useRef<number | undefined>(undefined)
  const clear = useCallback(() => { window.clearTimeout(timer.current); setMessage(null) }, [])
  const show = useCallback((m: ToastMessage) => {
    window.clearTimeout(timer.current)
    setMessage(m)
    timer.current = window.setTimeout(() => setMessage(null), TOAST_MS[m.kind])
  }, [])
  useEffect(() => {
    const on = (e: Event) => show((e as CustomEvent<ToastMessage>).detail)
    window.addEventListener(TOAST_EVENT, on)
    return () => { window.removeEventListener(TOAST_EVENT, on); window.clearTimeout(timer.current) }
  }, [show])
  // Ctrl+Z reverses an Undo message for as long as it shows.
  useEffect(() => {
    if (message?.kind !== 'undo') return
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'z') { e.preventDefault(); message.onAction?.(); clear() }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [message, clear])
  return (
    <ToastContext.Provider value={show}>
      {children}
      <div className="pointer-events-none fixed inset-x-0 bottom-10 z-40 flex justify-center">
        {message && <ToastView message={message} onAction={message.onAction} onClose={clear} />}
      </div>
    </ToastContext.Provider>
  )
}
