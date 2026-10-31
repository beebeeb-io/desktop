/**
 * The browser sign-in's state machine (task 1734; moved out of `WindowsFirstRun.tsx` by task 1683 slice 2, spec C
 * `docs/specs/2026-10-07-desktop-first-run-sign-in.md` §7.1 and C-S1), shared by the Windows first-run window and the
 * Beebeeb window on a Mac.
 *
 * The Rust `start_browser_login` handoff opens the system browser to the device-code page, waits for the browser to
 * send the session and the vault key back encrypted, installs both, and reports progress as `browser-login` events
 * (`connecting`, `waiting` with the code, `authorized`, `done`, `error`). This controller subscribes BEFORE it invokes (no
 * event is missed), shows every phase, and settles exactly once, from the command's answer.
 *
 * The answer is the outcome (spec C C-S1, task 1747 Part 1): the same `LoginOutcome` the password path returns, read
 * with `settledFrom`, the strict parser both paths share. A fresh sign-in delivered the keys, the same account is a
 * re-sign-in, and a different account (macOS and Linux; Windows refuses a second session) is the switch outcome with
 * 1887's count and ask snapshot: an outcome, never an error string, so no raw code reaches a screen and nothing waits for a
 * second signal. The events are progress only, and `done` settles nothing (the answer says what the sign-in became).
 */
import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { SIGN_IN_OUTCOME_UNREADABLE } from './accountSwitchCopy'
import { command, commandUnavailableLabel, type CommandResult, type DesktopLoginResult } from './desktopApi'
import { settledFrom, type SignInSettled } from './onboardingSignIn'

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
  phase: 'connecting' | 'waiting' | 'authorized' | 'done' | 'error'
  user_code?: string
  verification_uri?: string
  expires_in?: number
  /** The attempt the event belongs to: the id the controller passed to `start_browser_login`, echoed by Rust. */
  attempt?: string
  email?: string
  message?: string
}

export const BROWSER_LOGIN_EVENT = 'browser-login'
const DEFAULT_ERROR = 'Sign-in failed. Please try again.'

export interface BrowserSignInDeps {
  listen: (event: string, handler: (event: { payload: BrowserLoginEvent }) => void) => Promise<() => void>
  /** A fresh id for one attempt; Rust echoes it in every event of that attempt (review P3-2). */
  newAttemptId: () => string
  /** The handoff's answer: the `LoginOutcome` JSON, read with `settledFrom`. */
  start: (attempt: string) => Promise<CommandResult<DesktopLoginResult>>
}

let attemptsStarted = 0

export function browserSignInDeps(): BrowserSignInDeps {
  return {
    listen: (event, handler) => listen<BrowserLoginEvent>(event, handler),
    newAttemptId: () => `${Date.now().toString(36)}-${(attemptsStarted += 1).toString(36)}`,
    // Tauri reads the Rust parameter `attempt_id` as `attemptId`.
    start: (attempt) => command<DesktopLoginResult>('start_browser_login', { attemptId: attempt }),
  }
}

export class BrowserSignIn {
  state: BrowserSignInState = IDLE_BROWSER_SIGN_IN
  private running = false
  /** The attempt reached an outcome (settled or failed): later signals of it are ignored. */
  private terminal = false
  private disposed = false
  /** The screen went away; the attempt carries on to its end (`detach`). */
  private detached = false
  private unlisten: (() => void) | null = null
  /** The attempt whose events and answer this controller reads: each `start` has its own (review P3-2). */
  private attempt: string | null = null

  constructor(
    private readonly deps: BrowserSignInDeps,
    private readonly onState: (state: BrowserSignInState) => void,
    private readonly onSettled: (settled: SignInSettled) => void,
  ) {}

  async start(): Promise<void> {
    if (this.running || this.disposed) return
    this.running = true
    this.terminal = false
    const attempt = this.deps.newAttemptId()
    this.attempt = attempt
    this.set({ phase: 'connecting', userCode: null, verificationUri: null, error: null })
    // Subscribe before invoking so no progress event is missed.
    const unlisten = await this.deps.listen(BROWSER_LOGIN_EVENT, (event) => this.onEvent(event.payload))
    if (this.disposed) {
      unlisten()
      return
    }
    this.unlisten = unlisten
    const result = await this.deps.start(attempt)
    // An `error` event can end the attempt before the command answers. An answer of an attempt that is not the current
    // one (the person tried again after a failure) is nobody's: it settles nothing.
    if (this.disposed || this.terminal || this.attempt !== attempt) return
    if (result.ok) {
      // The same strict reading as the password path. A handoff has no second factor, so `requires_2fa` is a
      // contradiction here, and anything that is not the contract is an error, never a sign-in.
      const outcome = settledFrom(result.value)
      const asked2fa = (result.value as { requires_2fa?: unknown } | null | undefined)?.requires_2fa === true
      if (outcome.kind === 'unreadable' || asked2fa) this.fail(SIGN_IN_OUTCOME_UNREADABLE)
      else this.settle(outcome)
      return
    }
    // Any other outcome must leave "Connecting" and show a visible, actionable error: never a spinner with no way out.
    this.fail(result.unsupported ? commandUnavailableLabel('start_browser_login') : result.reason)
  }

  /**
   * The screen that started the attempt went away: a remount, or the person moved on. The attempt is NOT over. Rust
   * moves the session revision when it installs the session and answers some milliseconds later, and
   * `AccountSessionBoundary`'s one-second poll can remount the window in between; the person is signed in, so the answer
   * (it stands across a revision change: `ANSWER_STANDS_WHEN_THE_ACCOUNT_CHANGES`) must still move the flow on (to the
   * Finder screen), or the browser path, the main one, would often skip it. A detached controller is not told anything
   * any more; it stays listening only until its attempt ends.
   */
  detach(): void {
    this.detached = true
  }

  /** A hard stop: nothing is reported, nothing settles, nothing is listened to. */
  dispose(): void {
    this.disposed = true
    this.release()
  }

  private onEvent(payload: BrowserLoginEvent): void {
    if (this.disposed || this.terminal) return
    // Another attempt's event (a handoff that is still running in Rust, or timing out) is not this one's.
    if (this.attempt === null || payload.attempt !== this.attempt) return
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
        // Progress only: the answer says what the sign-in became (a re-sign-in is not a fresh sign-in).
        break
      case 'error':
        this.fail(payload.message ? payload.message : DEFAULT_ERROR)
        break
    }
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
    this.release()
  }

  private release(): void {
    const unlisten = this.unlisten
    this.unlisten = null
    unlisten?.()
  }

  private set(next: BrowserSignInState): void {
    this.state = next
    if (!this.disposed && !this.detached) this.onState(next)
  }
}

/**
 * The controller as a hook: a new controller for every mount (React's StrictMode mounts, unmounts and mounts again),
 * detached when the screen goes (the attempt itself is not stopped). `onSettled` may change between renders.
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
      // The card going away does not end the handoff (see `detach`): the last `onSettled` still carries it on.
      next.detach()
      controller.current = null
    }
  }, [])

  const start = () => controller.current?.start() ?? Promise.resolve()
  return { state, start }
}
