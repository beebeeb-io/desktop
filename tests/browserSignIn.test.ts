/**
 * The browser sign-in's state machine, shared by the Windows first-run window and the Beebeeb window on a Mac (task
 * 1683 slice 2; spec C `…-first-run-sign-in.md` §7.1 and C-S1, task 1734).
 *
 * It was inline in `WindowsFirstRun.tsx`. These tests hold what that code did (the order of subscribe and invoke, the
 * phases, the single `done`, the visible error) and what spec C asks of it for a Mac: a different account is the same
 * `SignInSettled` the password path yields, and the raw code `account_mismatch` never reaches a screen.
 *
 * The delicate part is the order of the two signals a different account sends: the `browser-login` event
 * `account_mismatch` (1887’s snapshot rides in it, pinned in `lib.rs` by `the_switch_count_reaches_the_sign_in_window`)
 * and the command’s `Err("account_mismatch")`. Either can arrive first.
 */
import { describe, expect, test } from 'bun:test'
import { BrowserSignIn, IDLE_BROWSER_SIGN_IN, MISMATCH_GRACE_MS, type BrowserSignInState } from '../src/browserSignIn'
import { SIGN_IN_OUTCOME_UNREADABLE } from '../src/accountSwitchCopy'
import type { SignInSettled } from '../src/onboardingSignIn'
import { commandUnavailableLabel, type CommandResult } from '../src/desktopApi'

const ASK = { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: ['/Users/sam/Changes not sent'] }

function rig(start: () => Promise<CommandResult<void>> | CommandResult<void> = () => ({ ok: true, value: undefined })) {
  const log: string[] = []
  const handlers = new Map<string, (event: { payload: any }) => void>()
  const timers: Array<{ fn: () => void; ms: number; live: boolean }> = []
  const states: BrowserSignInState[] = []
  const settled: SignInSettled[] = []
  let unlistened = 0
  const sign = new BrowserSignIn(
    {
      listen: async (name, handler) => {
        log.push(`listen ${name}`)
        handlers.set(name, handler)
        return () => { unlistened += 1 }
      },
      start: async () => {
        log.push('start')
        return start()
      },
      setTimer: (fn, ms) => { timers.push({ fn, ms, live: true }); return timers.length - 1 },
      clearTimer: (handle) => { timers[handle as number].live = false },
    },
    (state) => states.push(state),
    (result) => settled.push(result),
  )
  const emit = (payload: Record<string, unknown>) => handlers.get('browser-login')!({ payload })
  const fireTimers = () => timers.filter((t) => t.live).forEach((t) => { t.live = false; t.fn() })
  return { sign, log, emit, states, settled, timers, fireTimers, unlistened: () => unlistened }
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

describe('starting', () => {
  test('it subscribes BEFORE it invokes, so no progress event is ever missed', async () => {
    const r = rig()
    await r.sign.start()
    expect(r.log).toEqual(['listen browser-login', 'start'])
  })

  test('it is "connecting" at once, with nothing of an earlier attempt left on screen', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    expect(r.sign.state).toEqual({ phase: 'connecting', userCode: null, verificationUri: null, error: null })
  })

  test('a second start while one runs does nothing', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    await r.sign.start()
    expect(r.log.filter((line) => line === 'start')).toHaveLength(1)
  })

  test('the idle state is the one a screen starts from', () => {
    expect(IDLE_BROWSER_SIGN_IN).toEqual({ phase: 'idle', userCode: null, verificationUri: null, error: null })
  })
})

describe('progress', () => {
  test('waiting shows the device code and the address; authorized keeps them and says it is finishing', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'waiting', user_code: 'KQ7M-4TXP', verification_uri: 'https://app.beebeeb.io/cli-auth', expires_in: 300 })
    expect(r.sign.state).toEqual({ phase: 'waiting', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null })
    r.emit({ phase: 'authorized' })
    expect(r.sign.state).toEqual({ phase: 'authorized', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null })
  })

  test('a connecting event after waiting is shown as connecting (Rust restarts the handoff)', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'waiting', user_code: 'AAAA-BBBB' })
    r.emit({ phase: 'connecting' })
    expect(r.sign.state.phase).toBe('connecting')
  })

  test('every change is reported to the screen', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'waiting', user_code: 'AAAA-BBBB' })
    expect(r.states.map((s) => s.phase)).toEqual(['connecting', 'waiting'])
  })
})

describe('done: exactly once, whichever signal comes first', () => {
  test('the done event settles a fresh sign-in whose keys are here (C-S1: never the recovery phrase)', async () => {
    const r = rig()
    await r.sign.start()
    r.emit({ phase: 'done', email: 'sam@example.eu' })
    await tick()
    expect(r.settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
  })

  test('the command’s Ok settles too, when the event was missed', async () => {
    const r = rig()
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
  })

  test('the event first and the Ok after: once', async () => {
    let finish: (value: CommandResult<void>) => void = () => {}
    const r = rig(() => new Promise((resolve) => { finish = resolve }))
    const started = r.sign.start()
    await tick()
    r.emit({ phase: 'done' })
    finish({ ok: true, value: undefined })
    await started
    expect(r.settled).toHaveLength(1)
  })

  test('the Ok first and a late event after: once', async () => {
    const r = rig()
    await r.sign.start()
    r.emit({ phase: 'done' })
    expect(r.settled).toHaveLength(1)
  })

  test('it stops listening when it settles', async () => {
    const r = rig()
    await r.sign.start()
    expect(r.unlistened()).toBe(1)
  })
})

describe('errors are visible and recoverable (the handoff must never leave a spinner)', () => {
  test('an error event shows its message; with none, the standard sentence', async () => {
    const withMessage = rig(() => new Promise(() => {}))
    void withMessage.sign.start()
    await tick()
    withMessage.emit({ phase: 'error', message: 'The browser took too long.' })
    expect(withMessage.sign.state).toEqual({ phase: 'error', userCode: null, verificationUri: null, error: 'The browser took too long.' })
    const bare = rig(() => new Promise(() => {}))
    void bare.sign.start()
    await tick()
    bare.emit({ phase: 'error' })
    expect(bare.sign.state.error).toBe('Sign-in failed. Please try again.')
  })

  test('the first error ends the attempt: the Err that follows the event does not replace its message, and nothing settles', async () => {
    let fail: (value: CommandResult<void>) => void = () => {}
    const r = rig(() => new Promise((resolve) => { fail = resolve }))
    const started = r.sign.start()
    await tick()
    r.emit({ phase: 'error', message: 'The browser took too long.' })
    fail({ ok: false, reason: 'handoff failed', unsupported: false })
    await started
    expect(r.sign.state.error).toBe('The browser took too long.')
    expect(r.settled).toEqual([])
    expect(r.unlistened()).toBe(1)
  })

  test('a failed command leaves "connecting" with its reason (the Windows hang this was written for)', async () => {
    const r = rig(() => ({ ok: false, reason: 'Could not reach the server.', unsupported: false }))
    await r.sign.start()
    expect(r.sign.state.phase).toBe('error')
    expect(r.sign.state.error).toBe('Could not reach the server.')
    expect(r.settled).toEqual([])
  })

  test('a command this build does not have is said with the app’s one sentence for it (slice 5 rewrites that sentence in one place)', async () => {
    const r = rig(() => ({ ok: false, reason: 'command start_browser_login not found', unsupported: true }))
    await r.sign.start()
    expect(r.sign.state.error).toBe(commandUnavailableLabel('start_browser_login'))
  })

  test('after an error the person can try again from a clean state', async () => {
    let first = true
    const r = rig(() => (first ? ((first = false), { ok: false, reason: 'No.', unsupported: false }) : new Promise(() => {})))
    await r.sign.start()
    expect(r.sign.state.phase).toBe('error')
    void r.sign.start()
    await tick()
    expect(r.sign.state).toEqual({ phase: 'connecting', userCode: null, verificationUri: null, error: null })
  })
})

describe('C-S1: a different account is the password path’s outcome, and the raw code never reaches a screen', () => {
  const mismatch = { phase: 'account_mismatch', pending_changes: 3, unsent: ASK }

  test('the event settles the switch step with the count, the snapshot and no E3', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit(mismatch)
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 3, unsent: ASK, unsentError: null }])
  })

  test('a count that could not be read arrives as the coded E3 and is carried as such', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'account_mismatch', pending_changes: 0, unsent_error: 'unsent_count_failed: no' })
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 0, unsent: null, unsentError: 'unsent_count_failed: no' }])
  })

  test('a snapshot that is not whole is none, never a guess (the same strictness as the password path)', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'account_mismatch', pending_changes: 2, unsent: { askId: 'a', files: -1 } })
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 2, unsent: null, unsentError: null }])
  })

  test('a count that is not a whole number is not a switch with an invented count: it is an error, nothing settled', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'account_mismatch', pending_changes: 'many' })
    expect(r.settled).toEqual([])
    expect(r.sign.state.phase).toBe('error')
    expect(r.sign.state.error).toBe(SIGN_IN_OUTCOME_UNREADABLE)
  })

  test('the error event and the Err that follow carry the raw code: neither is shown, neither settles twice', async () => {
    let fail: (value: CommandResult<void>) => void = () => {}
    const r = rig(() => new Promise((resolve) => { fail = resolve }))
    const started = r.sign.start()
    await tick()
    r.emit(mismatch)
    r.emit({ phase: 'error', message: 'account_mismatch' })
    fail({ ok: false, reason: 'account_mismatch', unsupported: false })
    await started
    expect(r.settled).toHaveLength(1)
    expect(JSON.stringify(r.states)).not.toContain('account_mismatch')
    expect(r.sign.state.error).toBeNull()
  })

  test('the Err first, the event after (the two travel separately): the event’s snapshot is still used', async () => {
    let fail: (value: CommandResult<void>) => void = () => {}
    const r = rig(() => new Promise((resolve) => { fail = resolve }))
    const started = r.sign.start()
    await tick()
    fail({ ok: false, reason: 'account_mismatch', unsupported: false })
    await started
    expect(r.settled).toEqual([])
    expect(r.timers.some((t) => t.live && t.ms === MISMATCH_GRACE_MS)).toBe(true)
    // The `error` event that carries the raw code can arrive while the controller waits for the count: never shown.
    r.emit({ phase: 'error', message: 'account_mismatch' })
    expect(r.sign.state.error).toBeNull()
    expect(r.sign.state.phase).toBe('connecting')
    r.emit(mismatch)
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 3, unsent: ASK, unsentError: null }])
    r.fireTimers()
    expect(r.settled).toHaveLength(1)
  })

  test('the Err and no event at all: after the grace the switch step opens WITHOUT a snapshot, and Rust’s gate asks again before anything is signed out', async () => {
    const r = rig(() => ({ ok: false, reason: 'account_mismatch', unsupported: false }))
    await r.sign.start()
    expect(r.settled).toEqual([])
    r.fireTimers()
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 0, unsent: null, unsentError: null }])
    expect(r.sign.state.error).toBeNull()
  })

  test('only the exact code is a mismatch: a reason that merely contains it is an ordinary error', async () => {
    const r = rig(() => ({ ok: false, reason: 'not an account_mismatch, something else', unsupported: false }))
    await r.sign.start()
    expect(r.sign.state.phase).toBe('error')
    expect(r.settled).toEqual([])
  })
})

describe('leaving the screen', () => {
  test('dispose stops listening and ignores everything that arrives afterwards', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.sign.dispose()
    expect(r.unlistened()).toBe(1)
    r.emit({ phase: 'done' })
    expect(r.settled).toEqual([])
    expect(r.states.at(-1)?.phase).toBe('connecting')
  })

  test('dispose before the subscription landed still releases it', async () => {
    const r = rig()
    const started = r.sign.start()
    r.sign.dispose()
    await started
    expect(r.unlistened()).toBe(1)
    expect(r.settled).toEqual([])
  })

  test('dispose cancels a waiting mismatch timer', async () => {
    const r = rig(() => ({ ok: false, reason: 'account_mismatch', unsupported: false }))
    await r.sign.start()
    r.sign.dispose()
    r.fireTimers()
    expect(r.settled).toEqual([])
  })
})
