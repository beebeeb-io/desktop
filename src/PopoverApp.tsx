/**
 * The menu-bar popover (task 1683 slice 3; two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §6):
 * 372 × 488 pt under the b, on `popover_snapshot`. `popoverModel.ts` decides what shows; this file reads, renders and
 * runs the actions.
 *
 * Rules this file keeps (§6.4):
 * - **One error surface.** The popover never shows a toast. An action that fails replaces the body with its one
 *   sentence and "Try again"; the failure goes when the popover is shown again or the state moves on.
 * - **It opens no window but the Beebeeb window.** Window actions are `popover_open_window`; links go to the system
 *   browser through the opener plugin directly, never through `openUrl`, whose fallback is `window.open` (a webview of
 *   our own).
 * - **No session, no popover** (§6.2): if the session ends while it is open, it hides itself.
 */
import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react'
import { listen } from '@tauri-apps/api/event'
import { BILLING_URL, command, popoverSnapshot, type CommandResult } from './desktopApi'
import { finderOpenFailedToast, runFinderSetupAction } from './finderSetup'
import { FINDER_ACTION_FAILED } from './finderSetupCopy'
import { SnapshotReader } from './mainWindowModel'
import { POPOVER_HIDDEN_EVENT, POPOVER_SHOWN_EVENT, type PopoverPhase } from './popoverContract'
import { PopoverIcon } from './popoverIcons'
import { updateOffer } from './macUpdateOffer'
import {
  popoverView,
  type PopoverAction,
  type PopoverBody,
  type PopoverCommand,
  type PopoverInput,
  type PopoverScreen,
  type PopoverView,
  type RowView,
} from './popoverModel'

/** How often the popover reads the state again while it is shown. */
export const POPOVER_REFRESH_MS = 5000
/** A read still unanswered after this long is given up (the reader's watchdog, task 2179 item 6). */
export const POPOVER_WATCHDOG_MS = 15_000
/** "View online": the web app (the File menu's Open Web App opens the same address). */
export const WEB_APP_URL = 'https://app.beebeeb.io'
/** The one sentence a first read that failed shows, with Try again. */
export const READ_FAILED = 'Beebeeb couldn’t check its status.'

/** The one sentence a failed Open in Finder says on a Mac (task 17b: mapped once, by the helper), here without a toast. */
const OPEN_IN_FINDER_FAILED = ((message) => (typeof message === 'string' ? message : ''))(finderOpenFailedToast().message)

/** The sentence each action's failure shows, in place of the body (one error surface, §6.4). */
export const ACTION_FAILED: Readonly<Record<PopoverCommand, string>> = {
  open_review: 'Beebeeb couldn’t open its window.',
  open_beebeeb: 'Beebeeb couldn’t open its window.',
  download_update: 'Beebeeb couldn’t open your browser.',
  add_storage: 'Beebeeb couldn’t open your browser.',
  view_online: 'Beebeeb couldn’t open your browser.',
  resume: 'Beebeeb couldn’t resume sync.',
  retry_sync: 'Beebeeb couldn’t start syncing.',
  unlock: 'Beebeeb couldn’t unlock the vault.',
  open_location: OPEN_IN_FINDER_FAILED,
  finder_try_again: FINDER_ACTION_FAILED.try_again,
  finder_open_system_settings: FINDER_ACTION_FAILED.open_system_settings,
  finder_show_in_finder: FINDER_ACTION_FAILED.show_in_finder,
  finder_copy_details: FINDER_ACTION_FAILED.copy_details,
}

/** Actions whose result the state shows: the button stays busy until the refreshed snapshot is applied. */
export const REFRESHES: ReadonlySet<PopoverCommand> = new Set<PopoverCommand>([
  'resume', 'retry_sync', 'unlock', 'finder_try_again', 'finder_open_system_settings', 'finder_show_in_finder',
])

export interface ActionFailure {
  what: PopoverCommand
  /** The phase it failed in: a snapshot in another phase drops it. */
  phase: PopoverPhase
}

const openLink = (url: string) => command<void>('plugin:opener|open_url', { url })

/** Run one action. Each is one command; the answer is ok or the failure to show. */
export function runPopoverAction(what: PopoverCommand, update: PopoverInput['update']): Promise<CommandResult<unknown>> {
  switch (what) {
    case 'open_review':
      return command<void>('popover_open_window', { to: 'review' })
    case 'open_beebeeb':
      return command<void>('popover_open_window', { to: 'front' })
    case 'download_update':
      return update ? openLink(updateOffer(update.channel, update.version).url) : Promise.resolve({ ok: false, reason: 'The update offer has expired.', unsupported: false })
    case 'add_storage':
      return openLink(BILLING_URL)
    case 'view_online':
      return openLink(WEB_APP_URL)
    case 'resume':
      return command<void>('tray_resume_sync')
    case 'retry_sync':
      return command<void>('popover_retry_sync')
    case 'unlock':
      return command<string>('popover_unlock')
    case 'open_location':
      return command<void>('open_finder_location', { path: null })
    case 'finder_try_again':
      return runFinderSetupAction('try_again')
    case 'finder_open_system_settings':
      return runFinderSetupAction('open_system_settings')
    case 'finder_show_in_finder':
      return runFinderSetupAction('show_in_finder')
    case 'finder_copy_details':
      return runFinderSetupAction('copy_details')
  }
}

/** The view with a failed action's sentence in place of the body. Nothing on top, no status row. */
export function withFailure(view: PopoverView, failure: ActionFailure): PopoverView {
  const body: PopoverBody = {
    kind: 'message', tone: 'error', icon: 'alert', title: ACTION_FAILED[failure.what], lines: [], reason: null,
    action: { label: 'Try again', command: failure.what, primary: true },
  }
  return { ...view, update: null, attention: null, status: null, body, focus: failure.what }
}

/** The one snapshot read, shared with the Beebeeb window (`desktopApi.popoverSnapshot`, review P3-9 of #160). */
export const readPopoverSnapshot = (): Promise<CommandResult<PopoverInput>> => popoverSnapshot()

export function popoverReadFailure(cause: 'threw' | 'rejected' | 'timed out', error?: unknown): CommandResult<PopoverInput> {
  return { ok: false, reason: error instanceof Error ? error.message : cause, unsupported: false }
}

/** §9.1, §9.3: with no Dock icon there is no menu bar, so the popover answers Esc, ⌘, and ⌘Q itself. */
export function popoverKey(event: { key: string; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean; shiftKey?: boolean }): 'hide' | 'settings' | 'quit' | null {
  if (event.key === 'Escape' && !event.metaKey) return 'hide'
  if (!event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return null
  if (event.key === ',') return 'settings'
  if (event.key.toLowerCase() === 'q') return 'quit'
  return null
}

export const GEAR_ID = 'pop-gear'
export const actionId = (what: PopoverCommand) => `pop-action-${what}`

/** §11: focus starts on the first primary action, else the gear. */
export function focusIdFor(view: PopoverView | null): string {
  return view && view.focus !== 'gear' ? actionId(view.focus) : GEAR_ID
}

// ── Rendering ────────────────────────────────────────────────────────────────

interface SurfaceProps {
  screen: PopoverScreen | null
  readFailed: boolean
  busy: PopoverCommand | null
  onRun: (what: PopoverCommand) => void
  onGear: (anchor: { x: number; y: number }) => void
}

/** The hi-fi's button icons: Resume sync, Unlock vault and every Try again. */
const BUTTON_ICON: Partial<Record<PopoverCommand, string>> = { resume: 'play', unlock: 'unlock' }

function ActionButton({ action, busy, large, onRun }: { action: PopoverAction; busy: PopoverCommand | null; large?: boolean; onRun: (what: PopoverCommand) => void }) {
  const icon = BUTTON_ICON[action.command] ?? (action.label === 'Try again' ? 'refresh' : null)
  const className = ['pop-btn', action.primary ? 'pop-btn--primary' : '', large ? 'pop-btn--large' : ''].filter(Boolean).join(' ')
  return (
    <button id={actionId(action.command)} type="button" className={className} disabled={busy !== null} onClick={() => onRun(action.command)}>
      {large && icon ? <PopoverIcon name={icon} size={14} /> : null}
      {action.label}
    </button>
  )
}

function Row({ row }: { row: RowView }) {
  return (
    <div className="pop-row" role="listitem" aria-label={row.label}>
      <div className="pop-tile" aria-hidden="true"><PopoverIcon name={row.icon} /></div>
      <div className="pop-row-text">
        <div className="pop-name">{row.name}</div>
        <div className="pop-meta">{row.meta}</div>
        {row.progress ? (
          <div
            className={row.progress.encrypting ? 'pop-progress pop-progress--encrypting' : 'pop-progress'}
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={row.progress.percent ?? undefined}
            aria-valuetext={row.progress.valueText}
          >
            <i style={{ width: `${row.progress.percent ?? 0}%` }} />
          </div>
        ) : null}
      </div>
      {row.direction ? <span className="pop-dir"><PopoverIcon name={row.direction} size={14} label={row.direction === 'up' ? 'Uploaded' : 'Downloaded'} /></span> : null}
    </div>
  )
}

function Body({ body, busy, onRun }: { body: PopoverBody; busy: PopoverCommand | null; onRun: (what: PopoverCommand) => void }) {
  if (body.kind === 'activity') {
    return (
      <div className="pop-body">
        <div className="pop-label">{body.label}</div>
        <div role="list">{body.rows.map((row) => <Row key={row.id} row={row} />)}</div>
      </div>
    )
  }
  if (body.kind === 'syncing') {
    return (
      <div className="pop-body">
        <div className="pop-summary">
          <div className="pop-summary-top"><span>{body.left}</span>{body.total ? <span>{body.total}</span> : null}</div>
          {body.percent !== null ? <div className="pop-progress pop-progress--encrypting"><i style={{ width: `${body.percent}%` }} /></div> : null}
        </div>
        <div role="list">{body.active.map((row) => <Row key={row.id} row={row} />)}</div>
        {body.doneLabel ? <div className="pop-label">{body.doneLabel}</div> : null}
        {body.done.length > 0 ? <div role="list">{body.done.map((row) => <Row key={row.id} row={row} />)}</div> : null}
      </div>
    )
  }
  const disc = body.tone === 'busy'
    ? <div className="pop-spin pop-spin--live pop-spin--large" aria-hidden="true" />
    : <div className={`pop-disc pop-disc--${body.tone}`} aria-hidden="true">{body.icon ? <PopoverIcon name={body.icon} size={24} /> : null}</div>
  return (
    <div className="pop-body pop-message">
      {disc}
      <h2 className="pop-message-title">{body.title}</h2>
      {body.lines.map((line) => <p key={line} className="pop-message-line" aria-live={body.tone === 'busy' ? 'polite' : undefined}>{line}</p>)}
      {body.reason ? <div className="pop-reason">{body.reason}</div> : null}
      {body.action ? <ActionButton action={body.action} busy={busy} large onRun={onRun} /> : null}
    </div>
  )
}

export function PopoverSurface({ screen, readFailed, busy, onRun, onGear }: SurfaceProps): ReactElement {
  const gear = (
    <button
      id={GEAR_ID}
      type="button"
      className="pop-gear"
      aria-label="Beebeeb menu"
      onClick={(event) => {
        const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
        onGear({ x: rect.left, y: rect.bottom })
      }}
    >
      <PopoverIcon name="gear" size={18} />
    </button>
  )
  if (screen?.kind === 'no_session') return <div className="pop-root" role="dialog" aria-label="Beebeeb" />
  if (screen === null) {
    return (
      <div className="pop-root" role="dialog" aria-label="Beebeeb">
        <div className="pop-header"><div className="pop-account"><div className="pop-avatar" aria-hidden="true" /><div className="pop-account-text"><div className="pop-skeleton" /></div>{gear}</div></div>
        {readFailed ? (
          <div className="pop-body pop-message">
            <div className="pop-disc pop-disc--error" aria-hidden="true"><PopoverIcon name="alert" size={24} /></div>
            <h2 className="pop-message-title">{READ_FAILED}</h2>
            <button id={actionId('retry_sync')} type="button" className="pop-btn pop-btn--primary pop-btn--large" onClick={() => onRun('retry_sync')}>
              <PopoverIcon name="refresh" size={14} />
              Try again
            </button>
          </div>
        ) : (
          <div className="pop-body pop-message"><div className="pop-spin pop-spin--live pop-spin--large" role="status" aria-label="Loading" /></div>
        )}
        <Footer footer={{ open_location: false, view_online: false, add_storage: false }} busy={busy} onRun={onRun} />
      </div>
    )
  }
  const view = screen
  return (
    <div className="pop-root" role="dialog" aria-label="Beebeeb">
      <div className="pop-header">
        <div className="pop-account">
          <div className="pop-avatar" aria-hidden="true">{view.header.initial}</div>
          <div className="pop-account-text">
            <div className="pop-email">{view.header.email}</div>
            {view.header.storage ? <div className="pop-usage">{view.header.storage.label}</div> : null}
          </div>
          {gear}
        </div>
        {view.header.storage ? (
          <div className={view.header.storage.red ? 'pop-bar pop-bar--red' : 'pop-bar'}><i style={{ width: `${view.header.storage.percent}%` }} /></div>
        ) : null}
      </div>
      {view.update ? (
        <div className="pop-update">
          <PopoverIcon name="sync" size={18} />
          <div className="pop-update-text">{view.update.line}</div>
          <ActionButton action={view.update.action} busy={busy} onRun={onRun} />
          <div className="pop-update-hint">{view.update.hint}</div>
        </div>
      ) : null}
      {view.attention ? (
        <div className="pop-attention">
          <PopoverIcon name="alert" size={18} />
          <div className="pop-attention-text"><b>{view.attention.title}</b>{view.attention.text}</div>
          <ActionButton action={view.attention.action} busy={busy} onRun={onRun} />
        </div>
      ) : null}
      <Body body={view.body} busy={busy} onRun={onRun} />
      {view.status ? (
        <div className="pop-status">
          {view.status.kind === 'ok'
            ? <span className="pop-ok" aria-hidden="true"><PopoverIcon name="check" size={12} strokeWidth={2.4} /></span>
            : <span className="pop-spin pop-spin--live" aria-hidden="true" />}
          {view.status.label}
          {view.status.encrypted ? <span className="pop-encrypted"><PopoverIcon name="lock" size={14} />Encrypted</span> : null}
        </div>
      ) : null}
      <Footer footer={view.footer} busy={busy} onRun={onRun} />
    </div>
  )
}

function Footer({ footer, busy, onRun }: { footer: PopoverView['footer']; busy: PopoverCommand | null; onRun: (what: PopoverCommand) => void }) {
  const item = (what: 'open_location' | 'view_online' | 'add_storage', icon: string, label: string) => (
    <button id={actionId(what)} type="button" className="pop-footer-button" disabled={!footer[what] || busy !== null} onClick={() => onRun(what)}>
      <PopoverIcon name={icon} size={18} />
      {label}
    </button>
  )
  return (
    <div className="pop-footer">
      {item('open_location', 'folder', 'Open in Finder')}
      {item('view_online', 'globe', 'View online')}
      {item('add_storage', 'db', 'Add storage')}
    </div>
  )
}

// ── The popover ──────────────────────────────────────────────────────────────

export default function PopoverApp() {
  const [snapshot, setSnapshot] = useState<PopoverInput | null>(null)
  const [readFailed, setReadFailed] = useState(false)
  const [failure, setFailure] = useState<ActionFailure | null>(null)
  const [busy, setBusy] = useState<PopoverCommand | null>(null)
  const failureRef = useRef<ActionFailure | null>(null)
  const readerRef = useRef<SnapshotReader<CommandResult<PopoverInput>> | null>(null)

  const setFailureBoth = useCallback((next: ActionFailure | null) => {
    failureRef.current = next
    setFailure(next)
  }, [])

  if (readerRef.current === null) {
    readerRef.current = new SnapshotReader(
      readPopoverSnapshot,
      (result) => {
        if (!result.ok) {
          setReadFailed(true)
          return
        }
        setReadFailed(false)
        setSnapshot(result.value)
        if (failureRef.current && failureRef.current.phase !== result.value.phase) setFailureBoth(null)
      },
      { failure: popoverReadFailure, watchdogMs: POPOVER_WATCHDOG_MS },
    )
  }
  const reader = readerRef.current

  useEffect(() => {
    reader.start(POPOVER_REFRESH_MS)
    const unlisten: Array<() => void> = []
    let closed = false
    const on = (event: string, handler: () => void) => {
      void listen(event, handler).then((stop) => {
        if (closed) stop()
        else unlisten.push(stop)
      })
    }
    on(POPOVER_SHOWN_EVENT, () => {
      setFailureBoth(null)
      reader.resume()
      void reader.refresh()
    })
    // The window is hidden, never destroyed: reading stops while it is hidden (review P2-2 of #160).
    on(POPOVER_HIDDEN_EVENT, () => reader.stop())
    const onKey = (event: KeyboardEvent) => {
      const key = popoverKey(event)
      if (key === null) return
      event.preventDefault()
      if (key === 'hide') void command<void>('popover_hide')
      else if (key === 'settings') void command<void>('popover_open_window', { to: 'settings' })
      else void command<void>('popover_quit')
    }
    window.addEventListener('keydown', onKey)
    return () => {
      closed = true
      for (const stop of unlisten) stop()
      window.removeEventListener('keydown', onKey)
      reader.stop()
    }
  }, [reader, setFailureBoth])

  const screen = snapshot ? popoverView(snapshot, Math.floor(Date.now() / 1000)) : null
  const shown = screen?.kind === 'state' && failure ? withFailure(screen, failure) : screen

  useEffect(() => {
    if (screen?.kind === 'no_session') void command<void>('popover_hide')
  }, [screen?.kind])

  const focusTarget = shown && shown.kind === 'state' ? focusIdFor(shown) : null
  useEffect(() => {
    if (focusTarget && typeof document !== 'undefined') document.getElementById(focusTarget)?.focus()
  }, [focusTarget])

  const run = async (what: PopoverCommand) => {
    if (snapshot === null) {
      await reader.retry()
      return
    }
    setBusy(what)
    const result = await runPopoverAction(what, snapshot.update)
    if (result.ok) {
      setFailureBoth(null)
      if (REFRESHES.has(what)) await reader.refresh()
    } else {
      setFailureBoth({ what, phase: snapshot.phase })
    }
    setBusy(null)
  }

  return (
    <PopoverSurface
      screen={shown}
      readFailed={readFailed}
      busy={busy}
      onRun={(what) => void run(what)}
      onGear={(anchor) => void command<void>('popover_gear_menu', anchor)}
    />
  )
}
