/**
 * MacPopover: the macOS menu-bar popover (task 1683 slice 3).
 *
 * Spec `docs/specs/2026-09-30-macos-menubar-popover.md`, design
 * `design/hifi/macos-menubar-popover.jsx` (every artboard 1:1 in points). Fixed 372 x 488, radius
 * 12, an OPAQUE `--paper` page (Guus, 2026-09-30: public AppKit APIs only). Four regions:
 * header (76), body (316 in list states, 356 in message states), status row (40, list states
 * only), footer (56). The body is anchored from the top, so the title sits at the same y in every
 * message state.
 *
 * Mounted ONLY at `?window=popover&platform=macos` (`main.tsx`). No Tauri config creates that
 * window yet (slice 6 does), so no user sees this on any platform today.
 *
 * What is here, and what is not:
 *  - `model.ts` decides WHAT is shown (state, copy); this file only paints it.
 *  - `controller.ts` decides WHEN anything is fetched: on `popover-shown`, on `engine-status`
 *    while visible, never while hidden, never on a timer.
 *  - The gear opens a NATIVE menu owned by Rust (spec section 4); here it is a button that asks
 *    for it (`popover_gear_menu`, not implemented yet: `commands.ts`).
 *  - Amber appears in three roles only: encryption state (the lock art, the `Encrypted` lock,
 *    the hex), the fill of bytes being ENCRYPTED (uploads), and the one primary action.
 */
import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { command, openUrl, popoverSnapshot } from '../desktopApi'
import { ENGINE_STATUS_EVENT, POPOVER_SHOWN_EVENT } from '../popoverContract'
import { TrayActionButton, TrayIconButton } from '../trayShared'
import { initializeDesktopThemeFromConfig } from '../windows/theme'
import { systemLocale } from '../storageFormat'
import { MpIcon, MpSpinner } from './icons'
import { PopoverController, currentView, type Notice, type Ports } from './controller'
import type { ActionView, ActivityItemView, ArtId, BodyView, ConflictItemView, HeaderView, ListItem, PopoverView, StatusView, SummaryView } from './model'

const ACTIVITY_LIMIT = 10
export const POPOVER_WIDTH = 372
export const POPOVER_HEIGHT = 488

export function defaultPorts(): Ports {
  return {
    loadSnapshot: () => popoverSnapshot(ACTIVITY_LIMIT),
    call: (name, args) => command<unknown>(name, args),
    openUrl,
    hideWindow: () => getCurrentWindow().hide(),
    refreshTheme: () => initializeDesktopThemeFromConfig(),
  }
}

// ── Pieces ─────────────────────────────────────────────────────────────────────────────────

function Gear({ onOpen }: { onOpen: (x: number, y: number) => void }) {
  return (
    <TrayIconButton
      ariaLabel="Beebeeb menu"
      aria={{ 'aria-haspopup': 'menu' }}
      hoverBackground="var(--paper-2)"
      style={{ marginRight: -6, flexShrink: 0 }}
      onClick={(event) => {
        // The menu hangs from the gear's bottom-right corner, in points from the popover's top-left.
        const rect = event.currentTarget.getBoundingClientRect()
        onOpen(Math.round(rect.right), Math.round(rect.bottom))
      }}
    >
      <MpIcon name="settings" size={16} />
    </TrayIconButton>
  )
}

function Header({ header, onGear }: { header: HeaderView; onGear: (x: number, y: number) => void }) {
  const account = header.mode === 'account' ? header : null
  return (
    <div style={{ height: 76, flexShrink: 0, padding: '15px 16px 0', borderBottom: '1px solid var(--line)' }}>
      <div style={{ height: 33, display: 'flex', alignItems: 'center', gap: 10 }}>
        {account === null ? (
          <>
            <span className="mp-hex" aria-hidden="true" />
            <span style={{ fontSize: 15, fontWeight: 700, letterSpacing: '-0.03em' }}>beebeeb</span>
            <div style={{ marginLeft: 'auto', display: 'flex' }}>
              <Gear onOpen={onGear} />
            </div>
          </>
        ) : (
          <>
            <div
              aria-hidden="true"
              style={{
                width: 32,
                height: 32,
                borderRadius: '50%',
                background: 'var(--paper-3)',
                border: '1px solid var(--line)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: 13,
                fontWeight: 600,
                color: 'var(--ink-2)',
                flexShrink: 0,
              }}
            >
              {account.email ? account.email.charAt(0).toUpperCase() : <MpIcon name="user" size={16} />}
            </div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600, lineHeight: '18px', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }} title={account.email ?? undefined}>
                {account.email ?? 'Your account'}
              </div>
              {account.usage && <div style={{ fontSize: 11.5, lineHeight: '15px', color: 'var(--ink-3)' }}>Using {account.usage}</div>}
            </div>
            <Gear onOpen={onGear} />
          </>
        )}
      </div>
      {account?.usage && account.pct !== null && (
        <div
          role="progressbar"
          aria-label="Storage used"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={account.pct}
          style={{ marginTop: 9, height: 4, borderRadius: 2, background: 'var(--paper-3)', overflow: 'hidden' }}
        >
          <div style={{ width: `${account.pct}%`, height: '100%', background: account.full ? 'var(--red)' : 'var(--ink-3)' }} />
        </div>
      )}
    </div>
  )
}

function StatusRow({ status }: { status: StatusView }) {
  return (
    <div role="status" title={status.title ?? undefined} style={{ height: 40, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 8, padding: '0 16px', borderTop: '1px solid var(--line)', fontSize: 12.5, whiteSpace: 'nowrap' }}>
      {status.kind === 'ok' ? (
        <span aria-hidden="true" style={{ width: 16, height: 16, borderRadius: '50%', background: 'var(--ok-icon)', color: 'var(--paper)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <MpIcon name="check" size={11} />
        </span>
      ) : (
        <MpSpinner />
      )}
      <span style={{ fontWeight: 600 }}>{status.text}</span>
      <span style={{ marginLeft: 'auto', display: 'inline-flex', alignItems: 'center', gap: 5, color: 'var(--ink-3)', fontSize: 11.5 }}>
        <span aria-hidden="true" style={{ color: 'var(--amber-deep)', display: 'inline-flex' }}>
          <MpIcon name="lock" size={12} sw={1.8} />
        </span>
        Encrypted
      </span>
    </div>
  )
}

function Footer({ finderDisabled, onOpenFinder, onViewOnline, onAddStorage }: { finderDisabled: boolean; onOpenFinder: () => void; onViewOnline: () => void; onAddStorage: () => void }) {
  const common = { gap: 4, padding: '0', fontSize: 11.5, fontWeight: 600, hoverBackground: 'var(--paper-2)', disabledOpacity: 0.38, style: { flex: 1, minWidth: 0, lineHeight: 'normal' } } as const
  return (
    <div className="mp-footer" style={{ height: 56, flexShrink: 0, display: 'flex', borderTop: '1px solid var(--line)' }}>
      <TrayActionButton {...common} icon={<MpIcon name="folder" size={16} />} label="Open in Finder" onClick={onOpenFinder} disabled={finderDisabled} />
      <TrayActionButton {...common} icon={<MpIcon name="globe" size={16} />} label="View online" onClick={onViewOnline} />
      <TrayActionButton {...common} icon={<MpIcon name="drive" size={16} />} label="Add storage" onClick={onAddStorage} />
    </div>
  )
}

function PrimaryButton({ action, busy, onPress }: { action: ActionView; busy: boolean; onPress: () => void }) {
  return (
    <button type="button" className="mp-primary" disabled={action.disabled} aria-busy={busy || undefined} onClick={onPress}>
      {action.icon && <MpIcon name={action.icon} size={action.icon === 'play' ? 13 : 15} />}
      {action.label}
    </button>
  )
}

const ART_TONES = {
  neutral: { bg: 'var(--paper-2)', border: 'var(--line-2)', fg: 'var(--ink-3)' },
  amber: { bg: 'var(--amber-bg)', border: 'var(--amber-deep)', fg: 'var(--ink)' },
  err: { bg: 'var(--err-bg)', border: 'var(--err-line)', fg: 'var(--red)' },
} as const

/** The fixed 64 pt art slot: one style, a circle with one line glyph. */
function Art({ art, tone }: { art: ArtId; tone: keyof typeof ART_TONES }) {
  const t = ART_TONES[tone]
  const dashed = art === 'folderDashed' || art === 'spinnerDashed'
  const glyph =
    art === 'spinnerDashed' ? <MpSpinner size={26} /> : art === 'folderEnc' || art === 'folderDashed' ? <MpIcon name="folder" size={28} sw={1.5} /> : art === 'lock' ? <MpIcon name="lock" size={28} sw={1.7} /> : art === 'pause' ? <MpIcon name="pause" size={26} /> : art === 'alert' ? <MpIcon name="alert" size={28} sw={1.8} /> : art === 'wifiOff' ? <MpIcon name="wifiOff" size={28} sw={1.7} /> : art === 'drive' ? <MpIcon name="drive" size={28} sw={1.7} /> : art === 'shield' ? <MpIcon name="shield" size={28} sw={1.6} /> : <MpIcon name="users" size={28} sw={1.6} />
  return (
    <div aria-hidden="true" style={{ width: 64, height: 64, position: 'relative', flexShrink: 0 }}>
      <div style={{ width: 64, height: 64, borderRadius: '50%', background: t.bg, border: `1.5px ${dashed ? 'dashed' : 'solid'} ${t.border}`, color: t.fg, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{glyph}</div>
      {art === 'folderEnc' && (
        // Up to date, nothing recent: the folder plus the amber hex lock (encryption state).
        <svg width="64" height="64" viewBox="0 0 64 64" style={{ position: 'absolute', left: 0, top: 0 }}>
          <polygon points="50,38 60.39,44 60.39,56 50,62 39.61,56 39.61,44" fill="var(--amber)" stroke="var(--amber-deep)" strokeWidth="1.4" strokeLinejoin="round" />
          <rect x="46" y="49" width="8" height="6.4" rx="1.3" fill="var(--amber-ink)" />
          <path d="M47.6 49v-1.8a2.4 2.4 0 0 1 4.8 0v1.8" fill="none" stroke="var(--amber-ink)" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      )}
    </div>
  )
}

function PasswordForm({ wrong, busy, onSubmit }: { wrong: boolean; busy: boolean; onSubmit: (password: string) => void }) {
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    input.current?.focus()
  }, [])
  useEffect(() => {
    if (wrong) {
      input.current?.focus()
      input.current?.select()
    }
  }, [wrong])
  return (
    <form
      style={{ width: 292, marginTop: 14, textAlign: 'left' }}
      onSubmit={(event) => {
        event.preventDefault()
        const password = input.current?.value ?? ''
        if (password === '' || busy) return
        onSubmit(password)
        if (input.current) input.current.value = ''
      }}
    >
      <input ref={input} className="mp-input" type="password" aria-label="Vault password" aria-invalid={wrong || undefined} aria-describedby="mp-unlock-line" autoComplete="current-password" spellCheck={false} />
      <div id="mp-unlock-line" role="alert" style={{ height: 22, marginTop: 6, display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: 'var(--ink-2)' }}>
        {wrong && (
          <>
            <span style={{ color: 'var(--red)', display: 'inline-flex' }}>
              <MpIcon name="alert" size={14} sw={1.8} />
            </span>
            That password didn’t work. Try again.
          </>
        )}
      </div>
      <button type="submit" className="mp-primary" style={{ width: '100%', marginTop: 6 }} disabled={busy} aria-busy={busy || undefined}>
        Unlock
      </button>
    </form>
  )
}

function MessageBody({ body, busy, onAction, onUnlock }: { body: Extract<BodyView, { kind: 'message' }>; busy: boolean; onAction: (a: ActionView) => void; onUnlock: (password: string) => void }) {
  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center', padding: '56px 32px 0' }}>
      <div style={{ marginBottom: 16, display: 'flex' }}>
        <Art art={body.art} tone={body.artTone} />
      </div>
      <h1 data-mp-title style={{ margin: 0, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em', lineHeight: '21px' }}>
        {body.title}
      </h1>
      {body.copy && <p style={{ margin: '6px 0 0', fontSize: 12.5, color: 'var(--ink-2)', lineHeight: '18px', maxWidth: 292, textWrap: 'balance' }}>{body.copy}</p>}
      {body.detail && (
        <div title={body.detail} style={{ marginTop: 12, fontFamily: 'var(--font-mono)', fontSize: 11.5, lineHeight: '16px', color: 'var(--ink-3)', background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 6, padding: '3px 8px', maxWidth: 292, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {body.detail}
        </div>
      )}
      {body.form ? (
        <PasswordForm wrong={body.form.wrong} busy={busy} onSubmit={onUnlock} />
      ) : (
        body.action && (
          <div style={{ marginTop: 20 }}>
            <PrimaryButton action={body.action} busy={busy} onPress={() => onAction(body.action as ActionView)} />
          </div>
        )
      )}
    </div>
  )
}

function ListLabel({ children }: { children: ReactNode }) {
  return <h2 style={{ margin: 0, height: 32, flexShrink: 0, padding: '12px 16px 0', fontSize: 11.5, fontWeight: 600, color: 'var(--ink-3)' }}>{children}</h2>
}

function Tile({ icon }: { icon: 'file' | 'image' | 'versions' }) {
  return (
    <div aria-hidden="true" style={{ width: 32, height: 32, borderRadius: 8, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--ink-3)', flexShrink: 0 }}>
      <MpIcon name={icon} size={16} />
    </div>
  )
}

const ellipsis = { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' } as const

function ActivityRow({ item }: { item: ActivityItemView }) {
  return (
    <div role="group" aria-label={item.label} title={item.title} style={{ height: 56, padding: '0 16px', display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
      <Tile icon={item.tile} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, lineHeight: '18px', ...ellipsis }}>{item.name}</div>
        <div style={{ fontSize: 11.5, lineHeight: '15px', color: 'var(--ink-3)', ...ellipsis }}>{item.subline}</div>
        {item.progress !== null && (
          <div role="progressbar" aria-label={item.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={item.progress} style={{ marginTop: 4, height: 3, borderRadius: 2, background: 'var(--paper-3)', overflow: 'hidden' }}>
            <div style={{ width: `${item.progress}%`, height: '100%', background: item.progressTone === 'encrypting' ? 'var(--amber)' : 'var(--ink-3)' }} />
          </div>
        )}
      </div>
      {item.failed ? (
        <span aria-hidden="true" style={{ color: 'var(--red)', display: 'inline-flex' }}>
          <MpIcon name="alert" size={14} />
        </span>
      ) : (
        item.direction && (
          <span aria-hidden="true" style={{ color: 'var(--ink-3)', display: 'inline-flex' }}>
            <MpIcon name={item.direction === 'up' ? 'arrowUp' : 'arrowDown'} size={14} />
          </span>
        )
      )}
    </div>
  )
}

function ConflictRow({ item, onReview }: { item: ConflictItemView; onReview: () => void }) {
  return (
    <div role="group" aria-label={`${item.title}. ${item.subline}`} style={{ height: 56, padding: '0 16px', display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
      <Tile icon="versions" />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, lineHeight: '18px', ...ellipsis }}>{item.title}</div>
        <div style={{ fontSize: 11.5, lineHeight: '15px', color: 'var(--ink-3)', ...ellipsis }}>{item.subline}</div>
      </div>
      <button type="button" className="mp-secondary" onClick={onReview}>
        Review
      </button>
    </div>
  )
}

function Summary({ summary }: { summary: SummaryView }) {
  return (
    <div style={{ padding: '14px 16px 8px', flexShrink: 0 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12.5, lineHeight: '18px' }}>
        <span style={{ fontWeight: 600 }}>{summary.left}</span>
        {summary.right && <span style={{ color: 'var(--ink-3)' }}>{summary.right}</span>}
      </div>
      <div
        role="progressbar"
        aria-label="Overall progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={summary.pct ?? undefined}
        className={summary.pct === null ? 'mp-indeterminate' : undefined}
        style={{ marginTop: 6, height: 4, borderRadius: 2, background: 'var(--paper-3)', overflow: 'hidden' }}
      >
        {summary.pct !== null && <div style={{ width: `${summary.pct}%`, height: '100%', background: 'var(--ink-3)' }} />}
      </div>
    </div>
  )
}

function ListBody({ summary, items, onReview }: { summary: SummaryView | null; items: ListItem[]; onReview: (item: ConflictItemView) => void }) {
  const ref = useRef<HTMLDivElement>(null)
  // ArrowUp / ArrowDown / Home / End move through the list a row at a time. Attached with
  // addEventListener: the region is a scroll container, not a widget with its own events.
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const onKey = (event: KeyboardEvent) => {
      if (event.target !== el) return
      if (event.key === 'ArrowDown') el.scrollBy({ top: 56 })
      else if (event.key === 'ArrowUp') el.scrollBy({ top: -56 })
      else if (event.key === 'Home') el.scrollTo({ top: 0 })
      else if (event.key === 'End') el.scrollTo({ top: el.scrollHeight })
      else return
      event.preventDefault()
    }
    el.addEventListener('keydown', onKey)
    return () => el.removeEventListener('keydown', onKey)
  }, [])
  return (
    // A scrollable region has to be reachable from the keyboard (WCAG 2.1.1), so it takes a tab stop.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex
    <div ref={ref} className="mp-list" role="region" aria-label="Recent activity" tabIndex={0}>
      {summary && <Summary summary={summary} />}
      {items.map((item, i) =>
        item.kind === 'label' ? (
          <ListLabel key={`l${i}`}>{item.text}</ListLabel>
        ) : item.kind === 'conflict' ? (
          <ConflictRow key={`c${i}`} item={item} onReview={() => onReview(item)} />
        ) : (
          <ActivityRow key={item.id} item={item} />
        ),
      )}
    </div>
  )
}

function NoticeStrip({ notice }: { notice: Notice }) {
  return (
    <div role="alert" style={{ position: 'absolute', left: 0, right: 0, bottom: 0, padding: '8px 16px', display: 'flex', alignItems: 'center', gap: 8, background: 'var(--err-bg)', borderTop: '1px solid var(--err-line)', fontSize: 12, color: 'var(--ink)' }}>
      <span aria-hidden="true" style={{ color: 'var(--red)', display: 'inline-flex' }}>
        <MpIcon name="alert" size={14} sw={1.8} />
      </span>
      <span style={{ minWidth: 0, ...ellipsis }}>{notice.text}</span>
      {notice.detail && <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 11.5, color: 'var(--ink-3)', ...ellipsis }}>{notice.detail}</span>}
    </div>
  )
}

// ── The window ─────────────────────────────────────────────────────────────────────────────

function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]')]
}

export interface SurfaceHandlers {
  onAction: (action: ActionView) => void
  onGear: (x: number, y: number) => void
  onUnlock: (password: string) => void
  onReview: (item: ConflictItemView) => void
  onOpenFinder: () => void
  onViewOnline: () => void
  onAddStorage: () => void
}

/**
 * The painted popover for one `PopoverView`: no hooks of its own beyond the form and list
 * widgets, no controller, no Tauri. `MacPopover` below wires it to the controller; the tests
 * render it for every state.
 */
export function PopoverSurface({ view, notice, busy, handlers, rootRef, modality = 'pointer' }: { view: PopoverView; notice: Notice | null; busy: boolean; handlers: SurfaceHandlers; rootRef?: React.Ref<HTMLDivElement>; modality?: 'pointer' | 'keyboard' }) {
  return (
    <div ref={rootRef} className="mp-root" role="dialog" aria-label="Beebeeb" data-state={view.state} data-modality={modality} aria-busy={view.state === 'loading' || undefined}>
      <Header header={view.header} onGear={handlers.onGear} />
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden', position: 'relative' }}>
        {view.body.kind === 'list' && <ListBody summary={view.body.summary} items={view.body.items} onReview={handlers.onReview} />}
        {view.body.kind === 'message' && <MessageBody body={view.body} busy={busy} onAction={handlers.onAction} onUnlock={handlers.onUnlock} />}
        {notice && <NoticeStrip notice={notice} />}
      </div>
      {view.status && <StatusRow status={view.status} />}
      {view.footer.visible && <Footer finderDisabled={view.footer.finderDisabled} onOpenFinder={handlers.onOpenFinder} onViewOnline={handlers.onViewOnline} onAddStorage={handlers.onAddStorage} />}
    </div>
  )
}

export default function MacPopover({ ports }: { ports?: Ports }) {
  const [controller] = useState(() => new PopoverController(ports ?? defaultPorts()))
  const state = useSyncExternalStore(controller.subscribe, controller.getState)
  const root = useRef<HTMLDivElement>(null)
  // How the user last reached a control. A popover opened with a click puts focus on the gear (spec
  // section 10) without drawing a ring on it; the first Tab turns the rings on (macPopover.css).
  const [modality, setModality] = useState<'pointer' | 'keyboard'>('pointer')
  const view: PopoverView = currentView(state, Date.now() / 1000, systemLocale())

  // Rust's events and the window's focus. Registered once; they only ever call the controller,
  // and the controller makes no call while the popover is hidden.
  useEffect(() => {
    let disposed = false
    const unlisteners: Array<() => void> = []
    const keep = (pending: Promise<() => void>) => {
      pending.then((un) => (disposed ? un() : unlisteners.push(un))).catch(() => undefined)
    }
    keep(listen(POPOVER_SHOWN_EVENT, () => controller.shown()))
    keep(listen(ENGINE_STATUS_EVENT, () => controller.engineStatus()))
    keep(getCurrentWindow().onFocusChanged(({ payload: focused }) => { if (!focused) controller.hidden() }))
    return () => {
      disposed = true
      for (const un of unlisteners) un()
    }
  }, [controller])

  // Esc is handled on the window, not on an element (spec section 10).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        void controller.escape()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [controller])

  // On show, focus goes to the first control (the gear).
  useEffect(() => {
    if (state.shownCount === 0 || !root.current) return
    setModality('pointer')
    focusables(root.current)[0]?.focus()
  }, [state.shownCount])

  useEffect(() => {
    const toKeyboard = (event: KeyboardEvent) => {
      if (event.key === 'Tab' || event.key.startsWith('Arrow')) setModality('keyboard')
    }
    const toPointer = () => setModality('pointer')
    window.addEventListener('keydown', toKeyboard, true)
    window.addEventListener('pointerdown', toPointer, true)
    return () => {
      window.removeEventListener('keydown', toKeyboard, true)
      window.removeEventListener('pointerdown', toPointer, true)
    }
  }, [])

  // Tab stays inside the popover (spec section 10). Attached with addEventListener: the dialog is
  // a container, not a widget with events of its own.
  useEffect(() => {
    const el = root.current
    if (!el) return
    const onTab = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return
      const items = focusables(el)
      if (items.length === 0) return
      const first = items[0]
      const last = items[items.length - 1]
      const active = document.activeElement
      if (event.shiftKey && (active === first || !el.contains(active))) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && (active === last || !el.contains(active))) {
        event.preventDefault()
        first.focus()
      }
    }
    el.addEventListener('keydown', onTab)
    return () => el.removeEventListener('keydown', onTab)
  }, [])

  const handlers: SurfaceHandlers = {
    // `Unlock` (unlock_submit) is the password form's own submit, never a plain action button.
    onAction: (action) => {
      if (action.id !== 'unlock_submit') void controller.perform({ id: action.id })
    },
    onGear: (x, y) => void controller.perform({ id: 'gear', x, y }),
    onUnlock: (password) => void controller.perform({ id: 'unlock_submit', password }),
    onReview: (item) => void controller.perform({ id: 'review', fileId: item.fileId, fileName: item.fileName }),
    onOpenFinder: () => void controller.perform({ id: 'open_finder' }),
    onViewOnline: () => void controller.perform({ id: 'view_online' }),
    onAddStorage: () => void controller.perform({ id: 'add_storage' }),
  }

  return <PopoverSurface view={view} notice={state.notice} busy={state.busy !== null} handlers={handlers} rootRef={root} modality={modality} />
}
