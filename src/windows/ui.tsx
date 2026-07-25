/**
 * Shared Windows app UI primitives (extracted from WindowsApp PKG-SHELL) —
 * consumed by WindowsApp + src/windows/views/*.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type RefObject } from 'react'

// ── Design tokens ───────────────────────────────────────────────────────────

export const T = {
  paper: 'var(--paper)',
  paper2: 'var(--paper-2)',
  paper3: 'var(--paper-3)',
  line: 'var(--line)',
  line2: 'var(--line-2)',
  ink: 'var(--ink)',
  ink2: 'var(--ink-2)',
  ink3: 'var(--ink-3)',
  ink4: 'var(--ink-4)',
  amber: 'var(--amber)',
  amberDeep: 'var(--amber-deep)',
  amberBg: 'var(--amber-bg)',
  green: 'var(--green)',
  fontSans: 'var(--font-sans)',
  fontMono: 'var(--font-mono)',
} as const

// ── Mini icon set ────────────────────────────────────────────────────────────

export function NavIcon({ name, size = 13, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  switch (name) {
    case 'home':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M2.5 7 L8 2.5 L13.5 7 L13.5 13 L9.5 13 L9.5 9.5 L6.5 9.5 L6.5 13 L2.5 13 Z" />
        </svg>
      )
    case 'user':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="5.5" r="2.5" />
          <path d="M2 14 C2 11 4.5 9 8 9 C11.5 9 14 11 14 14" strokeLinecap="round" />
        </svg>
      )
    case 'cloud':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M4.5 11 C2.5 11 2 8 4 8 C4 5.5 7 4 9.5 5 C11 4 14 5.5 13.5 8 C15.5 8 15.5 11 13.5 11 Z" />
        </svg>
      )
    case 'folder':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M2 5 L2 13 L14 13 L14 6 L7 6 L5.5 4 L2 4 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'trash':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 4.5 L13 4.5" />
          <path d="M5.5 4.5 L5.5 3 L10.5 3 L10.5 4.5" />
          <path d="M4.2 4.5 L4.8 13.5 L11.2 13.5 L11.8 4.5" />
        </svg>
      )
    case 'bolt':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <path d="M9 2 L4 9 L8 9 L7 14 L12 7 L8 7 Z" />
        </svg>
      )
    case 'play':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <path d="M5 3 L13 8 L5 13 Z" />
        </svg>
      )
    case 'download':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M8 2 L8 10 M5 7 L8 10 L11 7" />
          <path d="M2 13 L14 13" />
        </svg>
      )
    case 'chart':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <path d="M2 13 L14 13" />
          <rect x="3.5" y="8" width="2.2" height="4" rx="0.5" fill={color} stroke="none" />
          <rect x="7" y="5" width="2.2" height="7" rx="0.5" fill={color} stroke="none" />
          <rect x="10.5" y="9.5" width="2.2" height="2.5" rx="0.5" fill={color} stroke="none" />
        </svg>
      )
    case 'device':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="2" y="3" width="9" height="7" rx="1.5" />
          <rect x="12" y="5" width="3" height="5" rx="1" />
          <line x1="2" y1="12" x2="11" y2="12" />
        </svg>
      )
    case 'shield':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M8 2 L13 4 L13 8 C13 11.5 10.5 13.5 8 14.5 C5.5 13.5 3 11.5 3 8 L3 4 Z" />
        </svg>
      )
    case 'clock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <circle cx="8" cy="8" r="6" />
          <path d="M8 5 L8 8 L10.5 9.5" />
        </svg>
      )
    case 'cog':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="8" r="2.5" />
          <path d="M8 1.5 L8 3.5 M8 12.5 L8 14.5 M1.5 8 L3.5 8 M12.5 8 L14.5 8 M3.5 3.5 L5 5 M11 11 L12.5 12.5 M12.5 3.5 L11 5 M5 11 L3.5 12.5" strokeLinecap="round" />
        </svg>
      )
    case 'lock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="3" y="7" width="10" height="7" rx="2" />
          <path d="M5 7 L5 5 C5 3.3 11 3.3 11 5 L11 7" />
        </svg>
      )
    case 'external':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M9 2 L14 2 L14 7 M14 2 L7.5 8.5" />
          <path d="M12 9.5 L12 13 L3 13 L3 4 L6.5 4" />
        </svg>
      )
    case 'check':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3.5 8.5 L6.5 11.5 L12.5 4.5" />
        </svg>
      )
    default:
      return <span style={{ display: 'block', width: s, height: s }} />
  }
}

// ── Shared primitives ───────────────────────────────────────────────────────

export function Chip({ children, tone = 'neutral' }: { children: React.ReactNode; tone?: 'neutral' | 'amber' | 'green' }) {
  const palette = {
    neutral: { bg: T.paper2, border: T.line2, color: T.ink3 },
    amber: { bg: T.amberBg, border: 'oklch(0.86 0.07 90)', color: 'oklch(0.4 0.08 72)' },
    green: { bg: 'oklch(0.96 0.04 155)', border: 'oklch(0.87 0.08 155)', color: 'oklch(0.4 0.1 155)' },
  }[tone]
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      padding: '1px 7px',
      fontSize: 10,
      fontFamily: T.fontMono,
      border: `1px solid ${palette.border}`,
      borderRadius: 999,
      background: palette.bg,
      color: palette.color,
      whiteSpace: 'nowrap' as const,
    }}>
      {children}
    </span>
  )
}

export function PrimaryBtn({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '8px 14px',
        fontSize: 12.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${T.ink}`,
        background: T.ink,
        color: T.paper,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.55 : 1,
        whiteSpace: 'nowrap' as const,
        letterSpacing: 0,
      }}
    >
      {children}
    </button>
  )
}

// A shimmering skeleton block — the canonical loading state for the data slots.
export function Skeleton({ width = '100%', height = 14, radius = 6 }: { width?: number | string; height?: number; radius?: number }) {
  return (
    <span
      aria-hidden
      style={{
        display: 'block',
        width,
        height,
        borderRadius: radius,
        background: `linear-gradient(90deg, ${T.paper3} 0%, ${T.paper2} 50%, ${T.paper3} 100%)`,
        backgroundSize: '200% 100%',
        animation: 'bb-shimmer 1.3s ease-in-out infinite',
      }}
    />
  )
}

// Reusable page header (title + subtitle).
export function PageHeader({ title, subtitle, aside }: { title: string; subtitle?: string; aside?: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 16, marginBottom: 20 }}>
      <div style={{ minWidth: 0 }}>
        <h1 style={{ margin: '0 0 6px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
          {title}
        </h1>
        {subtitle && <p style={{ margin: 0, fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 560 }}>{subtitle}</p>}
      </div>
      {aside}
    </div>
  )
}

export function Card({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ background: T.paper, border: `1px solid ${T.line}`, borderRadius: 10, boxShadow: 'var(--shadow-1)', ...style }}>
      {children}
    </div>
  )
}

// ── Toasts ──────────────────────────────────────────────────────────────────

export type ToastVariant = 'info' | 'success' | 'warning' | 'error'

export interface ToastTone {
  background: string
  border: string
  color: string
  iconBackground: string
}

export function toastToneForVariant(variant: ToastVariant): ToastTone {
  switch (variant) {
    case 'success':
      return {
        background: 'oklch(0.96 0.04 155)',
        border: 'oklch(0.84 0.08 155)',
        color: 'oklch(0.28 0.09 155)',
        iconBackground: 'oklch(0.44 0.11 155)',
      }
    case 'warning':
      return {
        background: 'oklch(0.97 0.025 255)',
        border: 'oklch(0.84 0.055 255)',
        color: 'oklch(0.31 0.08 255)',
        iconBackground: 'oklch(0.48 0.11 255)',
      }
    case 'error':
      return {
        background: 'oklch(0.98 0.02 25)',
        border: 'oklch(0.88 0.05 25)',
        color: 'oklch(0.42 0.15 25)',
        iconBackground: 'oklch(0.52 0.17 25)',
      }
    case 'info':
    default:
      return {
        background: T.paper,
        border: T.line2,
        color: T.ink2,
        iconBackground: T.ink3,
      }
  }
}

export interface ToastAction {
  label: string
  onClick: () => void | Promise<void>
  ariaLabel?: string
  disabled?: boolean
  ariaBusy?: boolean
}

export interface ToastInput {
  id?: string
  title?: React.ReactNode
  message: React.ReactNode
  variant?: ToastVariant
  action?: ToastAction
  durationMs?: number | null
  dismissible?: boolean
}

export interface ToastRecord {
  id: string
  title?: React.ReactNode
  message: React.ReactNode
  variant: ToastVariant
  action?: ToastAction
  durationMs: number | null
  dismissible: boolean
  createdAt: number
}

interface ToastContextValue {
  showToast: (toast: ToastInput) => string
  dismissToast: (id: string) => void
  clearToasts: () => void
}

const ToastContext = createContext<ToastContextValue | null>(null)
let toastIdCounter = 0

export function useToast(): ToastContextValue {
  const context = useContext(ToastContext)
  if (!context) {
    throw new Error('useToast must be used inside ToastProvider')
  }
  return context
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastRecord[]>([])

  const dismissToast = useCallback((id: string) => {
    setToasts((current) => current.filter((toast) => toast.id !== id))
  }, [])

  const clearToasts = useCallback(() => {
    setToasts([])
  }, [])

  const showToast = useCallback((input: ToastInput) => {
    const id = input.id ?? `toast-${Date.now()}-${toastIdCounter += 1}`
    const next: ToastRecord = {
      id,
      title: input.title,
      message: input.message,
      variant: input.variant ?? 'info',
      action: input.action,
      durationMs: input.durationMs === undefined ? 6000 : input.durationMs,
      dismissible: input.dismissible ?? true,
      createdAt: Date.now(),
    }

    setToasts((current) => {
      const existing = current.findIndex((toast) => toast.id === id)
      if (existing >= 0) {
        const updated = [...current]
        updated[existing] = next
        return updated
      }
      return [next, ...current].slice(0, 5)
    })

    return id
  }, [])

  const contextValue = useMemo(() => ({ showToast, dismissToast, clearToasts }), [showToast, dismissToast, clearToasts])

  return (
    <ToastContext.Provider value={contextValue}>
      {children}
      <div
        role="region"
        aria-label="Notifications"
        style={{
          position: 'fixed',
          top: 14,
          right: 14,
          zIndex: 1200,
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          width: 'min(360px, calc(100vw - 28px))',
          pointerEvents: 'none',
          fontFamily: T.fontSans,
        }}
      >
        {toasts.map((toast) => (
          <Toast key={toast.id} toast={toast} onDismiss={dismissToast} />
        ))}
      </div>
    </ToastContext.Provider>
  )
}

export function Toast({ toast, onDismiss }: { toast: ToastRecord; onDismiss: (id: string) => void }) {
  const tone = toastToneForVariant(toast.variant)
  const isError = toast.variant === 'error'

  useEffect(() => {
    if (toast.durationMs == null || toast.durationMs <= 0) return
    const timer = window.setTimeout(() => onDismiss(toast.id), toast.durationMs)
    return () => window.clearTimeout(timer)
  }, [toast.durationMs, toast.id, onDismiss])

  return (
    <div
      role={isError ? 'alert' : 'status'}
      aria-live={isError ? 'assertive' : 'polite'}
      style={{
        pointerEvents: 'auto',
        display: 'grid',
        gridTemplateColumns: '18px 1fr auto',
        gap: 10,
        alignItems: 'flex-start',
        padding: '11px 12px',
        background: tone.background,
        border: `1px solid ${tone.border}`,
        borderRadius: 8,
        boxShadow: 'var(--shadow-2)',
        color: tone.color,
      }}
    >
      <span
        aria-hidden
        style={{
          width: 18,
          height: 18,
          borderRadius: 999,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: tone.iconBackground,
          color: T.paper,
          fontSize: 12,
          fontWeight: 700,
          lineHeight: 1,
          marginTop: 1,
        }}
      >
        {toast.variant === 'success' ? <NavIcon name="check" size={11} color="currentColor" /> : toast.variant === 'info' ? 'i' : '!'}
      </span>
      <div style={{ minWidth: 0 }}>
        {toast.title && <div style={{ color: T.ink, fontSize: 12.5, fontWeight: 700, lineHeight: 1.35, marginBottom: 2 }}>{toast.title}</div>}
        <div style={{ fontSize: 12, lineHeight: 1.45, color: tone.color }}>{toast.message}</div>
        {toast.action && (
          <button
            type="button"
            onClick={() => void toast.action?.onClick()}
            disabled={toast.action.disabled}
            aria-label={toast.action.ariaLabel}
            aria-busy={toast.action.ariaBusy}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              height: 28,
              marginTop: 9,
              padding: '0 10px',
              borderRadius: 6,
              border: `1px solid ${T.ink}`,
              background: T.ink,
              color: T.paper,
              cursor: toast.action.disabled ? 'not-allowed' : 'pointer',
              opacity: toast.action.disabled ? 0.65 : 1,
              fontSize: 11.5,
              fontFamily: T.fontSans,
              fontWeight: 700,
              letterSpacing: 0,
            }}
          >
            {toast.action.label}
          </button>
        )}
      </div>
      {toast.dismissible && (
        <button
          type="button"
          onClick={() => onDismiss(toast.id)}
          aria-label="Dismiss notification"
          style={{
            width: 22,
            height: 22,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            borderRadius: 6,
            border: 'none',
            background: 'transparent',
            color: T.ink3,
            cursor: 'pointer',
            fontSize: 16,
            lineHeight: 1,
            padding: 0,
          }}
        >
          &times;
        </button>
      )}
    </div>
  )
}

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'textarea:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

function focusableElementsWithin(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter((element) => {
    if (element.tabIndex < 0) return false
    if (element.getAttribute('aria-hidden') === 'true') return false
    const style = window.getComputedStyle(element)
    return style.visibility !== 'hidden' && style.display !== 'none'
  })
}

export function modalFocusTargetIndex(currentIndex: number, count: number, direction: 'forward' | 'backward'): number {
  if (count <= 0) return -1
  if (currentIndex < 0 || currentIndex >= count) return direction === 'backward' ? count - 1 : 0
  return direction === 'backward'
    ? (currentIndex - 1 + count) % count
    : (currentIndex + 1) % count
}

// A small reusable modal primitive — extracted from the ad-hoc dialogs already
// used in WindowsApp.tsx (delete confirmation) and KnownFolderOnboarding.tsx
// (the fuller `role="dialog"` pattern this follows): fixed translucent
// backdrop, centered panel, scrollable body, footer close button. Backdrop
// click and Escape both close.
export function Modal({
  open,
  onClose,
  title,
  children,
  maxWidth = 480,
  ariaLabel,
  footer,
  initialFocusRef,
}: {
  open: boolean
  onClose: () => void
  title: React.ReactNode
  children: React.ReactNode
  maxWidth?: number
  ariaLabel?: string
  footer?: React.ReactNode
  initialFocusRef?: RefObject<HTMLElement | null>
}) {
  const fallbackFocusRef = useRef<HTMLButtonElement | null>(null)
  const dialogRef = useRef<HTMLDivElement | null>(null)
  const previousFocusRef = useRef<HTMLElement | null>(null)

  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose()
        return
      }
      if (event.key !== 'Tab') return
      const dialog = dialogRef.current
      if (!dialog) return
      const focusable = focusableElementsWithin(dialog)
      if (focusable.length === 0) {
        event.preventDefault()
        fallbackFocusRef.current?.focus()
        return
      }
      const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null
      const currentIndex = activeElement != null ? focusable.indexOf(activeElement) : -1
      const targetIndex = modalFocusTargetIndex(currentIndex, focusable.length, event.shiftKey ? 'backward' : 'forward')
      if (targetIndex < 0) return
      event.preventDefault()
      focusable[targetIndex]?.focus()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [open, onClose])

  useEffect(() => {
    if (!open) return
    previousFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const focusTimer = window.setTimeout(() => {
      const target = initialFocusRef?.current ?? fallbackFocusRef.current
      target?.focus()
    }, 0)
    return () => {
      window.clearTimeout(focusTimer)
      const previous = previousFocusRef.current
      if (previous && document.contains(previous)) previous.focus()
      previousFocusRef.current = null
    }
  }, [open, initialFocusRef])

  if (!open) return null

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- Escape already provides the keyboard-equivalent close action for this non-interactive backdrop click-outside-to-dismiss pattern.
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 1000,
        background: 'rgba(24, 20, 10, 0.38)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
        fontFamily: T.fontSans,
      }}
    >
      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-noninteractive-element-interactions -- This dialog container only stops backdrop clicks; Escape already provides the keyboard-equivalent close action. */}
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel ?? (typeof title === 'string' ? title : undefined)}
        onClick={(event) => event.stopPropagation()}
        style={{
          width: '100%',
          maxWidth,
          maxHeight: 'calc(100vh - 48px)',
          display: 'flex',
          flexDirection: 'column',
          background: T.paper,
          border: `1px solid ${T.line}`,
          borderRadius: 14,
          boxShadow: 'var(--shadow-3)',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            padding: '18px 20px',
            borderBottom: `1px solid ${T.line}`,
            flexShrink: 0,
          }}
        >
          <div style={{ fontSize: 15, fontWeight: 700, color: T.ink, letterSpacing: '-0.01em', minWidth: 0 }}>{title}</div>
          <button
            ref={fallbackFocusRef}
            type="button"
            onClick={onClose}
            aria-label="Close"
            style={{
              width: 26,
              height: 26,
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              borderRadius: 6,
              border: `1px solid ${T.line2}`,
              background: T.paper2,
              color: T.ink3,
              cursor: 'pointer',
              flexShrink: 0,
              fontSize: 14,
              lineHeight: 1,
            }}
          >
            &times;
          </button>
        </div>
        <div style={{ padding: '18px 20px', overflowY: 'auto', flex: 1, minHeight: 0 }}>{children}</div>
        {footer !== undefined ? (
          footer
        ) : (
          <div
            style={{
              display: 'flex',
              justifyContent: 'flex-end',
              padding: '12px 20px',
              borderTop: `1px solid ${T.line}`,
              flexShrink: 0,
            }}
          >
            <button
              type="button"
              onClick={onClose}
              style={{
                height: 30,
                padding: '0 14px',
                fontSize: 12,
                fontFamily: T.fontSans,
                fontWeight: 600,
                borderRadius: 6,
                border: `1px solid ${T.line2}`,
                background: T.paper,
                color: T.ink2,
                cursor: 'pointer',
              }}
            >
              Close
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
