/**
 * The popover's behaviour, with no React in it (task 1683 slice 3; spec section 9 "Popover
 * polling" and section 6 rules 3, 4 and 9).
 *
 * The rule this file exists to keep: NOTHING runs while the popover is hidden. The window is
 * created once and kept alive, so a timer or an eager fetch here would be exactly the polling
 * the old `tray` window was flagged for ("hidden, never shown, still polling"). The controller
 * therefore starts out hidden, makes no call at construction, and reacts to events only:
 *
 *   shown()        (Rust's `popover-shown`)  -> visible, theme re-read, ONE snapshot fetch
 *   engineStatus() (Rust's `engine-status`)  -> a snapshot fetch, but only while visible
 *   hidden()       (blur, Esc, any action)   -> nothing is fetched until the next shown()
 *
 * A fetch that is already running when another trigger arrives is coalesced into one trailing
 * fetch, so a burst of `engine-status` events (the pulse is 1 per second while files move)
 * never queues more than one extra request. There are no timers.
 *
 * Actions map to commands in `commands.ts`. A failed action is shown once, as `notice`, on the
 * popover itself (never a toast: a 372 pt window has no toast host, and spec section 7 wants
 * one surface). The password is passed through and never stored.
 */
import { BILLING_URL, type CommandResult } from '../desktopApi'
import type { PopoverSnapshot } from '../popoverContract'
import { PENDING_COMMANDS, VIEW_ONLINE_URL, WIRED_COMMANDS } from './commands'
import { DEFAULT_CONTEXT, loadFailView, loadingView, monoReason, viewForSnapshot, type ActionId, type PopoverView } from './model'

export interface Notice {
  text: string
  /** The mono reason line (<= 30 characters). */
  detail: string | null
}

export interface ControllerState {
  visible: boolean
  /** Bumped on every `shown()`; the component focuses the first control when it changes. */
  shownCount: number
  snapshot: PopoverSnapshot | null
  /** The last fetch failed. Only drawn when there is no earlier snapshot to keep showing. */
  loadFailed: boolean
  unlockOpen: boolean
  unlockWrong: boolean
  finderPending: boolean
  /** The action whose command is running (a second press of it is ignored). */
  busy: ActionRequest['id'] | null
  notice: Notice | null
}

export type ActionRequest =
  | { id: Exclude<ActionId, 'unlock_submit'> }
  | { id: 'unlock_submit'; password: string }
  | { id: 'open_finder' }
  | { id: 'view_online' }
  | { id: 'review'; fileId: string; fileName: string }
  | { id: 'gear'; x: number; y: number }

export interface Ports {
  loadSnapshot: () => Promise<CommandResult<PopoverSnapshot>>
  call: (name: string, args?: Record<string, unknown>) => Promise<CommandResult<unknown>>
  openUrl: (url: string) => Promise<CommandResult<void>>
  hideWindow: () => Promise<void>
  /** Re-read the theme preference (it can change in Settings while the popover is hidden). */
  refreshTheme: () => Promise<void>
}

const INITIAL: ControllerState = {
  visible: false,
  shownCount: 0,
  snapshot: null,
  loadFailed: false,
  unlockOpen: false,
  unlockWrong: false,
  finderPending: false,
  busy: null,
  notice: null,
}

export const WRONG_PASSWORD_REASON = 'wrong_password'

/**
 * A command Rust does not have. `command()` flags the wording it knows (`unsupported`), but
 * Tauri v2 says `Command <name> not found`, which that check does not match, so it is matched here.
 */
function isMissingCommand(result: Extract<CommandResult<unknown>, { ok: false }>): boolean {
  return result.unsupported || /command\s+\S+\s+not found/i.test(result.reason)
}

export class PopoverController {
  private current: ControllerState = INITIAL
  private readonly listeners = new Set<() => void>()
  private refreshing = false
  private again = false

  constructor(private readonly ports: Ports) {}

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  readonly getState = (): ControllerState => this.current

  private set(patch: Partial<ControllerState>): void {
    this.current = { ...this.current, ...patch }
    for (const listener of this.listeners) listener()
  }

  /** Rust showed the window. */
  shown(): void {
    this.set({ visible: true, shownCount: this.current.shownCount + 1, unlockOpen: false, unlockWrong: false, notice: null })
    void this.ports.refreshTheme().catch(() => undefined)
    void this.refresh()
  }

  /** The window lost focus or was hidden. No call is made for it, and none is made until `shown()`. */
  hidden(): void {
    if (this.current.visible) this.set({ visible: false })
  }

  /** Esc, handled on the window: hide it. */
  async escape(): Promise<void> {
    await this.hide()
  }

  /** An `engine-status` event. Ignored while hidden. */
  engineStatus(): void {
    if (this.current.visible) void this.refresh()
  }

  private async hide(): Promise<void> {
    this.set({ visible: false })
    await this.ports.hideWindow().catch(() => undefined)
  }

  /** One snapshot fetch, coalescing overlapping triggers into one trailing fetch. */
  async refresh(): Promise<void> {
    if (!this.current.visible) return
    if (this.refreshing) {
      this.again = true
      return
    }
    this.refreshing = true
    try {
      do {
        this.again = false
        const result = await this.ports.loadSnapshot()
        if (result.ok) this.set({ snapshot: result.value, loadFailed: false })
        else this.set({ loadFailed: true })
      } while (this.again && this.current.visible)
    } finally {
      this.refreshing = false
    }
  }

  async perform(request: ActionRequest): Promise<void> {
    if (request.id === 'unlock_open') {
      this.set({ unlockOpen: true, unlockWrong: false, notice: null })
      return
    }
    if (request.id === 'reload') {
      this.set({ notice: null })
      await this.refresh()
      return
    }
    if (this.current.busy === request.id) return
    this.set({ busy: request.id, notice: null })
    try {
      await this.run(request)
    } finally {
      this.set({ busy: null })
    }
  }

  private fail(text: string, result: Extract<CommandResult<unknown>, { ok: false }>): void {
    this.set({ notice: { text, detail: monoReason(isMissingCommand(result) ? 'not available in this build' : result.reason) } })
  }

  /** Run a command; on success hide the popover (every action that leaves it does), else show why not. */
  private async leave(result: CommandResult<unknown>, failure: string): Promise<void> {
    if (result.ok) await this.hide()
    else this.fail(failure, result)
  }

  private async run(request: ActionRequest): Promise<void> {
    const { call, openUrl } = this.ports
    switch (request.id) {
      case 'unlock_submit': {
        const result = await call(PENDING_COMMANDS.unlockWithPassword, { password: request.password })
        if (result.ok) {
          this.set({ unlockOpen: false, unlockWrong: false })
          await this.refresh()
        } else if (!isMissingCommand(result) && result.reason.includes(WRONG_PASSWORD_REASON)) {
          this.set({ unlockWrong: true })
        } else {
          this.set({ unlockWrong: false })
          this.fail('Couldn’t unlock the vault.', result)
        }
        return
      }
      case 'resume': {
        const result = await call(WIRED_COMMANDS.resumeSync)
        if (result.ok) await this.refresh()
        else this.fail('Couldn’t resume sync.', result)
        return
      }
      case 'retry': {
        const result = await call(PENDING_COMMANDS.retrySync)
        // No retry command yet: re-reading the status is all `Try again` can honestly do.
        if (!result.ok && !isMissingCommand(result)) this.fail('Couldn’t try again.', result)
        await this.refresh()
        return
      }
      case 'finder_add':
      case 'finder_retry': {
        this.set({ finderPending: true })
        const result = await call(WIRED_COMMANDS.installFinder, { path: null })
        this.set({ finderPending: false })
        // `install_finder_location` saves its failure and returns an Ok state (task 1683 slice 5,
        // decision D1); the refreshed snapshot is what shows f2. Only a rejection has no saved state.
        if (!result.ok) this.fail('Couldn’t add Beebeeb to Finder.', result)
        await this.refresh()
        return
      }
      case 'open_system_settings':
        await this.leave(await call(WIRED_COMMANDS.openSystemSettings), 'Couldn’t open System Settings.')
        return
      case 'setup':
      case 'sign_in':
        await this.leave(await call(WIRED_COMMANDS.openOnboarding), 'Couldn’t open Beebeeb.')
        return
      case 'add_storage':
        await this.leave(await openUrl(BILLING_URL), 'Couldn’t open the page.')
        return
      case 'view_online':
        await this.leave(await openUrl(VIEW_ONLINE_URL), 'Couldn’t open the page.')
        return
      case 'open_finder':
        await this.leave(await call(WIRED_COMMANDS.openFinder, { path: null }), 'Couldn’t open Finder.')
        return
      case 'review':
        await this.leave(await call(WIRED_COMMANDS.openConflict, { fileId: request.fileId, fileName: request.fileName, isText: false }), 'Couldn’t open the review.')
        return
      case 'gear': {
        // The native menu is shown by Rust; the popover must stay (spec section 4: NEEDS-MAC).
        const result = await call(PENDING_COMMANDS.gearMenu, { x: request.x, y: request.y })
        if (!result.ok) this.fail('Couldn’t open the menu.', result)
        return
      }
      default:
        return
    }
  }
}

/** The view for the controller's current state. `now` is unix seconds. */
export function currentView(state: ControllerState, now: number, locale: string): PopoverView {
  if (state.snapshot === null) return state.loadFailed ? loadFailView() : loadingView()
  return viewForSnapshot(state.snapshot, {
    ...DEFAULT_CONTEXT,
    now,
    locale,
    unlockOpen: state.unlockOpen,
    unlockWrong: state.unlockWrong,
    finderPending: state.finderPending,
  })
}
