/**
 * Pure logic behind the macOS Beebeeb window (task 1683 slice 1; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6, §7.8, §8.2, §9.3, §11).
 *
 * The window is one webview with one sidebar. The sidebar shows Home (Review and Trash join in slice 4) or, after
 * the gear, ⌘, or a menu, the Settings list in System Settings style (ruling 1). Every opener reaches the window
 * through one request (`MainNav`, Rust's `main:navigate`), and `navigate` is the only place that turns a request
 * into a view. Nothing here touches React or Tauri, so `tests/mainWindowModel.test.ts` pins each decision.
 */
import { SETTINGS_TABS, listAfterKey, parseSettingsTab, monoReason, type SettingsTab } from './macSettingsModel'
import type { PopoverSnapshot } from './popoverContract'

/** The window's own sidebar, top to bottom. Review (⌘2) and Trash (⌘3) arrive with their panes in slice 4. */
export const MAIN_SECTIONS = [{ id: 'home', label: 'Home', icon: 'home' }] as const
export type MainSection = (typeof MAIN_SECTIONS)[number]['id']

/** The Settings list, top to bottom (§7.8): the account row is Account, then General, Sync, Security and, apart, About. */
const SETTINGS_SIGNED_IN: readonly SettingsTab[] = ['account', 'general', 'sync', 'security', 'about']
/** Signed out, the sections that need no session (§7.8). */
const SETTINGS_SIGNED_OUT: readonly SettingsTab[] = ['general', 'about']

export function settingsSections(signedIn: boolean): readonly SettingsTab[] {
  return signedIn ? SETTINGS_SIGNED_IN : SETTINGS_SIGNED_OUT
}

export type MainView =
  | { mode: 'main'; section: MainSection }
  /** `returnTo` is what the back control names and goes back to. */
  | { mode: 'settings'; section: SettingsTab; returnTo: MainSection }

/** One request to the window: from the gear, the menus (⌘1, ⌘[, ⌘,), the sign-out flow and the update check. */
export type MainNav =
  | { to: 'home' }
  | { to: 'settings'; section: SettingsTab | null }
  | { to: 'back' }
  /** “Sign in again”, asked for by an opener (`open_reauth_window`), whatever the session says (review P3-1). */
  | { to: 'reauth' }

export interface NavContext {
  signedIn: boolean
  /** The last Settings section shown in this window, which ⌘, opens again. */
  lastSettings: SettingsTab | null
  /** §8.2: a window-modal sheet is open. */
  sheetOpen: boolean
}

const HOME: MainView = { mode: 'main', section: 'home' }

/**
 * Where a request lands. While a sheet is open nothing moves (§8.2: ⌘1, ⌘[ and ⌘, cannot unmount Settings ›
 * Account under its sheet; the menu items are disabled as well, this is the window's own half). Signed out, a
 * section that needs a session is never shown: General instead.
 */
export function navigate(view: MainView, nav: MainNav, ctx: NavContext): MainView {
  if (ctx.sheetOpen) return view
  switch (nav.to) {
    case 'home':
    case 'reauth':
      return HOME
    case 'back':
      return view.mode === 'settings' ? { mode: 'main', section: view.returnTo } : view
    case 'settings': {
      const allowed = settingsSections(ctx.signedIn)
      const wanted = nav.section ?? ctx.lastSettings
      const section = wanted !== null && allowed.includes(wanted) ? wanted : 'general'
      const returnTo = view.mode === 'settings' ? view.returnTo : view.section
      return { mode: 'settings', section, returnTo }
    }
  }
}

/**
 * The sections that exist only with a session: signed out only General and About do (§7.8). Account is the one section
 * that is not in this list although signed out it has no sidebar entry: it hosts the sign-out's outcome ("You’re signed
 * out" and the held sentence) until the window remounts on the sign-in card, and coercing it would take that outcome off the
 * screen at the moment the sign-out finishes.
 */
const ONLY_WITH_A_SESSION: ReadonlyArray<SettingsTab> = ['security', 'sync']

/**
 * §7.8, signed out: only General and About exist. A request is coerced by `navigate`; this is the other half: a session that
 * ends while Settings shows a section that exists only with one (the account signs out, the session is revoked) puts the
 * window on General instead of leaving a body up that reads an account it no longer has (PR #140 review, P2-6 and its
 * delta review). The same object comes back when there is nothing to change.
 */
export function coerceSettingsView(view: MainView, signedIn: boolean): MainView {
  if (signedIn || view.mode !== 'settings' || !ONLY_WITH_A_SESSION.includes(view.section)) return view
  return { ...view, section: 'general' }
}

/** A `main:navigate` request as Rust serialises it; anything else is no request. */
export function parseMainNav(raw: unknown): MainNav | null {
  if (typeof raw !== 'object' || raw === null) return null
  const value = raw as { to?: unknown; section?: unknown }
  if (value.to === 'home' || value.to === 'back' || value.to === 'reauth') return { to: value.to }
  if (value.to !== 'settings') return null
  if (value.section === undefined || value.section === null) return { to: 'settings', section: null }
  const section = typeof value.section === 'string' ? parseSettingsTab(value.section) : null
  return section === null ? null : { to: 'settings', section }
}

/**
 * Where a sign-out's held sentence (`accountSession.ts`) is shown when it becomes held while this window is mounted. A
 * signed-in window that shows no card goes to Settings › Account, where the note lives. A window that shows a sign-in
 * card (including the account switch, which holds its own sentence while its card is up) shows the sentence above that
 * card (`SignInWindow`'s note) and is not moved (review P2-1).
 */
export function heldNoteGoesToSettings({ signedIn, showingCard }: { signedIn: boolean; showingCard: boolean }): boolean {
  return signedIn && !showingCard
}

/** The back control's words (§7.8, §10.2 #48): where it returns to, or "Sign in" while signed out. */
export function backLabel(returnTo: MainSection, signedIn: boolean): string {
  if (!signedIn) return 'Sign in'
  return MAIN_SECTIONS.find((s) => s.id === returnTo)?.label ?? 'Home'
}

/** The toolbar title: the section's name (§7.8). */
export function sectionTitle(view: MainView): string {
  if (view.mode === 'main') return MAIN_SECTIONS.find((s) => s.id === view.section)?.label ?? 'Home'
  return SETTINGS_TABS.find((tab) => tab.id === view.section)?.label ?? 'Settings'
}

/** The id of the button a section is drawn as, in either list. */
export const settingsButtonId = (section: SettingsTab) => `mw-settings-${section}`
export const mainButtonId = (section: MainSection) => `mw-section-${section}`
export const GEAR_ID = 'mw-gear'

/**
 * Focus after a request (§11): entering Settings, or another Settings section, focuses that section; leaving it by
 * the back control or ⌘[ focuses the gear; ⌘1 focuses the section it chose. A request that changed nothing moves
 * nothing.
 */
export function focusAfterNav(previous: MainView, next: MainView, nav: MainNav): string | null {
  if (previous === next) return null
  if (next.mode === 'settings') return settingsButtonId(next.section)
  if (nav.to === 'back' && previous.mode === 'settings') return GEAR_ID
  return mainButtonId(next.section)
}

/**
 * The keys of either sidebar list (§7.6, §11): ↑ and ↓ move and select and wrap, Home and End jump. The automatic
 * activation the shipped Settings tabs used (`tabAfterKey`), with vertical arrows. Any other key: null.
 */
export { listAfterKey }

/** What Rust needs to enable the menu items (§8.2, F15): Back only while Settings shows, nothing under a sheet. */
export function menuStateFor(view: MainView, sheetOpen: boolean): { settingsShowing: boolean; sheetOpen: boolean } {
  return { settingsShowing: view.mode === 'settings', sheetOpen }
}

/** The first view: Home, or the Settings section a `?tab=` deep link names (kept for the render tools). */
export function initialMainView(search: string): MainView {
  const tab = parseSettingsTab(new URLSearchParams(search).get('tab'))
  return tab === null ? HOME : { mode: 'settings', section: tab, returnTo: 'home' }
}

// ── Home › status (§7.6: the popover's state in one row) ─────────────────────

export interface HomeStatus {
  tone: 'ok' | 'busy' | 'neutral' | 'alert'
  title: string
  detail: string | null
  /** The mono reason line of the error state, at most 30 characters. */
  reason: string | null
  /** The one action this row offers, where a command for it exists. */
  action: 'resume' | 'add_storage' | null
}

type StatusInput = Pick<PopoverSnapshot, 'phase' | 'paused' | 'storage_full' | 'pending_changes' | 'reason'> & {
  engine: Pick<PopoverSnapshot['engine'], 'state' | 'files_remaining' | 'last_tick_ok_at'>
}

const FINDER_PHASES: ReadonlySet<PopoverSnapshot['phase']> = new Set(['finder_failed', 'finder_user_disabled', 'finder_adding', 'finder_missing'])

/**
 * The sync part of the snapshot when its phase is a Finder one. The backend's `popover_phase` ranks every Finder phase
 * above paused, storage full, offline and error (`surfaces/phase.rs`), so under a Finder phase those come from the
 * snapshot's own fields, in the same order. Follow-up to PR #134 (auto-review thread 2): before, Home said "Up to date"
 * from the engine alone and hid a pause and its Resume sync.
 */
function syncPhase(input: StatusInput): PopoverSnapshot['phase'] {
  if (!FINDER_PHASES.has(input.phase)) return input.phase
  if (input.paused || input.engine.state === 'paused') return 'paused'
  if (input.storage_full) return 'storage_full'
  if (input.engine.state === 'offline') return 'offline'
  if (input.engine.state === 'error') return 'error'
  return 'syncing' // `syncState` tells syncing from up to date by the engine
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

/** "Last checked 2 min ago". Null when the last check is unknown: no placeholder. */
export function lastCheckedLine(lastTickOkAt: number | null, nowSeconds: number): string | null {
  if (lastTickOkAt === null) return null
  const ago = Math.max(0, nowSeconds - lastTickOkAt)
  if (ago < 60) return 'Last checked just now'
  if (ago < 3600) return `Last checked ${Math.floor(ago / 60)} min ago`
  return `Last checked ${Math.floor(ago / 3600)} h ago`
}

function syncState(input: StatusInput, nowSeconds: number): HomeStatus {
  const left = input.engine.files_remaining
  if (input.engine.state === 'syncing' || (left !== null && left > 0)) {
    return {
      tone: 'busy',
      title: 'Syncing',
      detail: left !== null && left > 0 ? `${left} ${plural(left, 'file', 'files')} left` : null,
      reason: null,
      action: null,
    }
  }
  return { tone: 'ok', title: 'Up to date', detail: lastCheckedLine(input.engine.last_tick_ok_at, nowSeconds), reason: null, action: null }
}

/**
 * The row's words per phase, from the popover's states (§6.3). Under a Finder phase it says the sync part, read from
 * the snapshot's fields (`syncPhase`), because Home's Finder row says the Finder part: both show. Signed out, session
 * ended and locked never reach here: the window shows its sign-in card or the locked card instead.
 */
export function homeStatus(input: StatusInput, nowSeconds: number): HomeStatus {
  const n = input.pending_changes
  switch (syncPhase(input)) {
    case 'syncing':
      return syncState(input, nowSeconds)
    case 'paused':
      return {
        tone: 'neutral',
        title: 'Sync is paused',
        detail: n > 0
          ? `${n} ${plural(n, 'change on this Mac is', 'changes on this Mac are')} waiting. Nothing syncs until you resume.`
          : 'Nothing syncs until you resume.',
        reason: null,
        action: 'resume',
      }
    case 'offline':
      return { tone: 'alert', title: 'Can’t reach Beebeeb', detail: 'Check your connection. Files on this Mac are unchanged.', reason: null, action: null }
    case 'error':
      return {
        tone: 'alert',
        title: 'Sync stopped',
        detail: 'Beebeeb couldn’t finish syncing. Files on this Mac are unchanged.',
        reason: monoReason(input.reason?.code),
        action: null,
      }
    case 'storage_full':
      return {
        tone: 'alert',
        title: 'Your storage is full',
        detail: `${n} ${plural(n, 'change', 'changes')} can’t upload until you free up space or add storage. They stay on this Mac.`,
        reason: null,
        action: 'add_storage',
      }
    default:
      return syncState(input, nowSeconds)
  }
}

// ── The request channel (Rust `main:navigate`) ───────────────────────────────

/**
 * Rust stores ONE pending request (the latest) and emits this event; the window then pulls it with
 * `consume_main_navigation`. A window that Rust builds for the request is not listening yet when it is told, so the
 * window also pulls once as soon as its listener is registered (the pattern of the update check and the sign-out
 * flow request).
 */
export const MAIN_NAVIGATE_EVENT = 'main:navigate'

type Listen = (event: string, callback: () => void) => Promise<() => void>
type Pull = () => Promise<{ ok: true; value: unknown } | { ok: false }>

export function connectMainNavigation(listen: Listen, pull: Pull, apply: (nav: MainNav) => void): () => void {
  let stopped = false
  let unlisten: (() => void) | null = null
  const drain = async () => {
    const result = await pull()
    if (stopped || !result.ok) return
    const nav = parseMainNav(result.value)
    if (nav !== null) apply(nav)
  }
  void listen(MAIN_NAVIGATE_EVENT, () => void drain()).then((remove) => {
    if (stopped) {
      remove()
      return
    }
    unlisten = remove
    void drain()
  })
  return () => {
    stopped = true
    unlisten?.()
  }
}

// ── Reading the snapshot (follow-up to PR #134, auto-review thread 1) ─────────

type ReadResult = { ok: boolean }

/**
 * Reads the account and sync state on a timer without ever throwing a read away because the timer fired. One read
 * is in flight at a time: a tick while one is pending starts nothing (`popover_snapshot` can take up to 8 s, the
 * storage fetch's timeout, and the tick is 5 s). `refresh` is for an outcome that changed the state (a sign-out, an
 * unlock): it reads at once, even with a read in flight, and every read started before it is dropped when it
 * finishes, whether before or after the refresh's own, so the outcome's read wins and the state from before it is
 * never shown again. Only a refresh runs two reads at once, so the newest result is always the one applied. `stop`
 * (the window unmounted) applies nothing more; `resume` undoes it (StrictMode remounts).
 */
export class SnapshotReader<R extends ReadResult> {
  private started = 0
  private floor = 0
  private pending = 0
  private stopped = true

  constructor(
    private readonly read: () => Promise<R>,
    private readonly onResult: (result: R) => void,
  ) {}

  resume(): void {
    this.stopped = false
  }

  stop(): void {
    this.stopped = true
  }

  /** A timer tick, or the first read: skipped while a read is in flight. */
  tick(): void {
    if (this.stopped || this.pending > 0) return
    this.start()
  }

  /** After an outcome: read now, and let nothing started before this win over it. */
  refresh(): void {
    if (this.stopped) return
    this.floor = this.start()
  }

  private start(): number {
    const sequence = ++this.started
    this.pending += 1
    const done = (result: R) => {
      this.pending -= 1
      if (this.stopped || sequence < this.floor) return
      this.onResult(result)
    }
    void this.read().then(done, () => done({ ok: false } as R))
    return sequence
  }
}
