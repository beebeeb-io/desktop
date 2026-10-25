/**
 * The browser sign-in's state machine (task 1734; moved out of `WindowsFirstRun.tsx` by task 1683 slice 2, spec C
 * `docs/specs/2026-10-07-desktop-first-run-sign-in.md` §7.1 and C-S1), shared by the Windows first-run window and the
 * Beebeeb window on a Mac.
 *
 * The Rust `start_browser_login` handoff opens the system browser to the device-code page, waits for the browser to
 * send the session and the vault key back encrypted, installs both, and reports progress as `browser-login` events
 * (`connecting`, `waiting` with the code, `authorized`, `done`, `error`). The command itself resolves when the handoff
 * is over. This controller subscribes BEFORE it invokes (no event is missed), shows every phase, and settles exactly
 * once: whichever of the `done` event and the command's `Ok` comes first.
 *
 * A different account (macOS and Linux; Windows refuses a second session): Rust sends the event `account_mismatch`
 * with 1887's count and ask snapshot in it, then an `error` event and an `Err` that both carry the raw code
 * `account_mismatch`. Spec C C-S1 asks for one outcome on both sign-in paths and no raw code on a screen. Rust returning
 * a `LoginOutcome` would edit a 1887 pin (`the_switch_count_reaches_the_sign_in_window` holds the event), so the
 * controller does it on this side: the event becomes the same `SignInSettled` the password path yields, and the two raw
 * signals are swallowed. The event and the `Err` travel separately, so either can come first; when the `Err` is first
 * the controller waits `MISMATCH_GRACE_MS` for the event, and if it never comes the switch step opens WITHOUT a
 * snapshot, which is safe: its confirm then asks Rust to sign out with no `keepUnsent`, and Rust's gate refuses while
 * changes wait and the step counts again before anything is signed out.
 */
import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { SIGN_IN_OUTCOME_UNREADABLE } from './accountSwitchCopy'
import { command, commandUnavailableLabel, type CommandResult } from './desktopApi'
import { unsentFrom, type SignInSettled } from './onboardingSignIn'

export type BrowserPhase = 'idle' | 'connecting' | 'waiting' | 'authorized' | 'error'

export interface BrowserSignInState {
  phase: BrowserPhase
  /** The device code the person types into the page the browser opened. */
  userCode: string | null
  /** The page's address, shown when the browser did not open. */
  verificationUri: string | null
  error: string | null
}

export const IDLE_BROWSER_SIGN_IN: BrowserSignInState = { phase: 'idle', userCode: null, verificationUri: null, error: null }

/** The `browser-login` event's payload (`browser_login.rs`). */
export interface BrowserLoginEvent {
  phase: 'connecting' | 'waiting' | 'authorized' | 'done' | 'account_mismatch' | 'error'
  user_code?: string
  verification_uri?: string
  expires_in?: number
  email?: string
  message?: string
  pending_changes?: unknown
  unsent?: unknown
  unsent_error?: unknown
}

export const BROWSER_LOGIN_EVENT = 'browser-login'
/** How long the `Err("account_mismatch")` waits for the event that carries the count, if it came first. */
export const MISMATCH_GRACE_MS = 750
/** The raw code Rust uses for a different account; it never reaches a screen. */
const MISMATCH_CODE = 'account_mismatch'
const DEFAULT_ERROR = 'Sign-in failed. Please try again.'

export interface BrowserSignInDeps {
  listen: (event: string, handler: (event: { payload: BrowserLoginEvent }) => void) => Promise<() => void>
  start: () => Promise<CommandResult<void>>
  setTimer: (fn: () => void, ms: number) => unknown
  clearTimer: (handle: unknown) => void
}

export function browserSignInDeps(): BrowserSignInDeps {
  return {
    listen: (event, handler) => listen<BrowserLoginEvent>(event, handler),
    start: () => command<void>('start_browser_login'),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  }
}

export class BrowserSignIn {
  state: BrowserSignInState = IDLE_BROWSER_SIGN_IN
  private running = false
  /** The attempt reached an outcome (settled or failed): later signals of it are ignored. */
  private terminal = false
  private disposed = false
  private unlisten: (() => void) | null = null
  private mismatchTimer: unknown = null

  constructor(
    private readonly deps: BrowserSignInDeps,
    private readonly onState: (state: BrowserSignInState) => void,
    private readonly onSettled: (settled: SignInSettled) => void,
  ) {}

  async start(): Promise<void> {
    if (this.running || this.disposed) return
    this.running = true
    this.terminal = false
    this.set({ phase: 'connecting', userCode: null, verificationUri: null, error: null })
    // Subscribe before invoking so no progress event is missed.
    const unlisten = await this.deps.listen(BROWSER_LOGIN_EVENT, (event) => this.onEvent(event.payload))
    if (this.disposed) {
      unlisten()
      return
    }
    this.unlisten = unlisten
    const result = await this.deps.start()
    // The done event (or a different account's) can settle the attempt before the command answers.
    if (this.disposed || this.terminal) return
    if (result.ok) {
      this.settle({ kind: 'fresh', vaultUnlocked: true })
      return
    }
    if (result.reason === MISMATCH_CODE) {
      // The event that carries the count may still be on its way.
      this.mismatchTimer = this.deps.setTimer(() => {
        this.mismatchTimer = null
        this.settleMismatch(null)
      }, MISMATCH_GRACE_MS)
      return
    }
    // Any other outcome must leave "Connecting" and show a visible, actionable error: never a spinner with no way out.
    this.fail(result.unsupported ? commandUnavailableLabel('start_browser_login') : result.reason)
  }

  dispose(): void {
    this.disposed = true
    this.cancelMismatchTimer()
    this.release()
  }

  private onEvent(payload: BrowserLoginEvent): void {
    if (this.disposed || this.terminal) return
    switch (payload.phase) {
      case 'connecting':
        this.set({ ...this.state, phase: 'connecting' })
        break
      case 'waiting':
        this.set({
          phase: 'waiting',
          userCode: payload.user_code ? payload.user_code : this.state.userCode,
          verificationUri: payload.verification_uri ? payload.verification_uri : this.state.verificationUri,
          error: this.state.error,
        })
        break
      case 'authorized':
        this.set({ ...this.state, phase: 'authorized' })
        break
      case 'done':
        this.settle({ kind: 'fresh', vaultUnlocked: true })
        break
      case 'account_mismatch':
        this.settleMismatch(payload)
        break
      case 'error':
        // A different account's `error` carries the raw code; the mismatch is the outcome, not this.
        if (payload.message === MISMATCH_CODE) break
        this.fail(payload.message ? payload.message : DEFAULT_ERROR)
        break
    }
  }

  /** A different account. `payload` is the event's, or null when only the `Err` came (see the file header). */
  private settleMismatch(payload: BrowserLoginEvent | null): void {
    if (this.disposed || this.terminal) return
    this.cancelMismatchTimer()
    if (payload === null) {
      this.settle({ kind: 'account_mismatch', pendingChanges: 0, unsent: null, unsentError: null })
      return
    }
    const pending = payload.pending_changes
    if (typeof pending !== 'number' || !Number.isSafeInteger(pending) || pending < 0) {
      // A count that is not a whole number is no switch with an invented count (the password path's rule).
      this.fail(SIGN_IN_OUTCOME_UNREADABLE)
      return
    }
    this.settle({
      kind: 'account_mismatch',
      pendingChanges: pending,
      unsent: unsentFrom(payload.unsent),
      unsentError: typeof payload.unsent_error === 'string' ? payload.unsent_error : null,
    })
  }

  private settle(settled: SignInSettled): void {
    this.finish()
    this.onSettled(settled)
  }

  private fail(message: string): void {
    this.finish()
    this.set({ ...this.state, phase: 'error', error: message })
  }

  /** The attempt is over: nothing more is listened to, and a new attempt may start. */
  private finish(): void {
    this.terminal = true
    this.running = false
    this.cancelMismatchTimer()
    this.release()
  }

  private cancelMismatchTimer(): void {
    if (this.mismatchTimer === null) return
    this.deps.clearTimer(this.mismatchTimer)
    this.mismatchTimer = null
  }

  private release(): void {
    const unlisten = this.unlisten
    this.unlisten = null
    unlisten?.()
  }

  private set(next: BrowserSignInState): void {
    this.state = next
    if (!this.disposed) this.onState(next)
  }
}

/**
 * The controller as a hook: a new controller for every mount (React's StrictMode mounts, unmounts and mounts again, and
 * a disposed controller stays disposed), released when the screen goes. `onSettled` may change between renders.
 */
export function useBrowserSignIn({ onSettled }: { onSettled: (settled: SignInSettled) => void }) {
  const [state, setState] = useState<BrowserSignInState>(IDLE_BROWSER_SIGN_IN)
  const settledRef = useRef(onSettled)
  settledRef.current = onSettled
  const controller = useRef<BrowserSignIn | null>(null)

  useEffect(() => {
    const next = new BrowserSignIn(browserSignInDeps(), setState, (settled) => settledRef.current(settled))
    controller.current = next
    return () => {
      next.dispose()
      controller.current = null
    }
  }, [])

  const start = () => controller.current?.start() ?? Promise.resolve()
  return { state, start }
}
