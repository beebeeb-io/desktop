/**
 * The browser sign-in's state machine, shared by the Windows first-run window and the Beebeeb window on a Mac (task
 * 1683 slice 2; spec C `…-first-run-sign-in.md` §7.1 and C-S1, task 1734).
 *
 * Task 1747 Part 1 (C-S1, the Rust half): `start_browser_login` returns the password path's `LoginOutcome`, and this
 * controller settles from that answer through `settledFrom`, the strict parser the password path uses, so both paths reach
 * the account switch step through one parser. The `browser-login` events are progress only. Nothing here keeps a timer or
 * a raw code: a different account is an outcome, never an error string, and the event-or-`Err` race of slice 2 is gone.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { BrowserSignIn, browserSignInDeps, IDLE_BROWSER_SIGN_IN, type BrowserSignInState } from '../src/browserSignIn'
import { SIGN_IN_OUTCOME_UNREADABLE } from '../src/accountSwitchCopy'
import { settledFrom, type SignInSettled } from '../src/onboardingSignIn'
import { commandUnavailableLabel, type CommandResult } from '../src/desktopApi'
import { rustStr } from './fixtures/rustConstants'

const ASK = { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: ['/Users/sam/Changes not sent'] }

/** What Rust's `LoginOutcome` serialises to; the controller reads it with `settledFrom`, as the password path does. */
const outcome = (over: Record<string, unknown> = {}) => ({
  requires_2fa: false,
  reauthenticated: false,
  vault_unlocked: true,
  key_replaced: false,
  account_mismatch: null,
  ...over,
})
const FRESH = outcome()
const SAME_ACCOUNT = outcome({ reauthenticated: true })
const MISMATCH = outcome({ vault_unlocked: false, account_mismatch: { pending_changes: 3, unsent: ASK } })
type Answer = CommandResult<unknown>
const ok = (value: unknown = FRESH): Answer => ({ ok: true, value })

function rig(start: () => Promise<Answer> | Answer = () => ok()) {
  const log: string[] = []
  const handlers = new Map<string, (event: { payload: any }) => void>()
  const states: BrowserSignInState[] = []
  const settled: SignInSettled[] = []
  let unlistened = 0
  let attemptCount = 0
  const attempts: string[] = []
  const sign = new BrowserSignIn(
    {
      listen: async (name, handler) => {
        log.push(`listen ${name}`)
        handlers.set(name, handler)
        return () => { unlistened += 1 }
      },
      newAttemptId: () => `attempt-${++attemptCount}`,
      start: async (attempt) => {
        log.push('start')
        attempts.push(attempt)
        return start()
      },
    },
    (state) => states.push(state),
    (result) => settled.push(result),
  )
  // Rust echoes the attempt id it was given in every event (review P3-2); an event is the current attempt's unless a
  // test says otherwise.
  const emit = (payload: Record<string, unknown>, attempt: string | null = `attempt-${attemptCount}`) =>
    handlers.get('browser-login')!({ payload: attempt === null ? payload : { attempt, ...payload } })
  return { sign, log, emit, states, settled, attempts, unlistened: () => unlistened }
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

// Review P3-2: Rust ran no attempt id on a Mac or Linux, so attempt 1's late timeout `error` event was read as attempt 2's.
// Attempt 1 is still waiting in Rust when the person leaves the browser panel for the email form and comes back (a new
// panel, a new controller, a new subscription); when attempt 1 times out, its `error` reached attempt 2's subscriber and
// failed it. Every attempt now carries an id the controller generated: Rust echoes it in each event, and the controller
// reads only its own.
describe('attempts (P3-2)', () => {
  test('the controller names the attempt when it invokes, and every attempt has its own id', async () => {
    const r = rig(() => Promise.resolve({ ok: false, reason: 'x', unsupported: false }))
    await r.sign.start()
    await r.sign.start()
    expect(r.attempts).toEqual(['attempt-1', 'attempt-2'])
  })

  test('an event of another attempt is ignored: attempt 1’s timeout cannot fail attempt 2', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'waiting', user_code: 'KQ7M-4TXP', verification_uri: 'https://app.beebeeb.io/cli-auth', expires_in: 300 })
    r.emit({ phase: 'error', message: 'Timed out waiting for the browser' }, 'attempt-0')
    r.emit({ phase: 'done', email: 'a@example.eu' }, 'attempt-0')
    r.emit({ phase: 'account_mismatch', pending_changes: 1 }, 'attempt-0')
    r.emit({ phase: 'waiting', user_code: 'OTHER-CODE' }, 'attempt-0')
    expect(r.sign.state).toEqual({ phase: 'waiting', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null })
    expect(r.settled).toEqual([])
  })

  test('an event with no attempt at all is nobody’s and is ignored too', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'error', message: 'from an unnamed attempt' }, null)
    expect(r.sign.state.phase).toBe('connecting')
  })

  test('the current attempt’s own events still count', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'error', message: 'The browser took too long.' })
    expect(r.sign.state).toEqual({ phase: 'error', userCode: null, verificationUri: null, error: 'The browser took too long.' })
  })

  test('a late answer of attempt 1’s command does not settle or fail attempt 2', async () => {
    let answerFirst: (result: Answer) => void = () => undefined
    let calls = 0
    const r = rig(() => (++calls === 1 ? new Promise<Answer>((resolve) => { answerFirst = resolve }) : new Promise(() => {})))
    void r.sign.start()
    await tick()
    r.emit({ phase: 'error', message: 'It did not work' }) // attempt 1 fails; the person tries again
    expect(r.sign.state.phase).toBe('error')
    void r.sign.start()
    await tick()
    expect(r.sign.state.phase).toBe('connecting')
    // Attempt 1's command finally answers, with its own failure.
    answerFirst({ ok: false, reason: 'Timed out waiting for the browser', unsupported: false })
    await tick()
    expect(r.sign.state.phase).toBe('connecting')
    expect(r.sign.state.error).toBeNull()
    expect(r.settled).toEqual([])
  })
})

describe('the real deps name the attempt the way Rust reads it (P3-2)', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })

  // Tauri turns a command's `attempt_id` argument into the JavaScript key `attemptId`. A key the command does not know is
  // dropped without a word, so the id never reached Rust and every event came back without one (a mutation run found
  // exactly that: nothing exercised the real invoke). The Rust parameter's name is read from the source here.
  test('start_browser_login is invoked with { attemptId }, the camelCase of the Rust parameter attempt_id', async () => {
    const calls: Array<{ name: string; args: any }> = []
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async (name: string, args: any) => { calls.push({ name, args }); return null } } }
    await browserSignInDeps().start('lq3x-1')
    expect(calls).toEqual([{ name: 'start_browser_login', args: { attemptId: 'lq3x-1' } }])
    const rust = readFileSync(new URL('../src-tauri/src/browser_login.rs', import.meta.url), 'utf8')
    expect(rust).toContain('attempt_id: Option<String>')
  })

  test('each id the real deps make is its own', () => {
    const deps = browserSignInDeps()
    const ids = new Set([deps.newAttemptId(), deps.newAttemptId(), browserSignInDeps().newAttemptId()])
    expect(ids.size).toBe(3)
    for (const id of ids) expect(id).toMatch(/^[A-Za-z0-9_-]{1,64}$/)
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

describe('the command’s answer is the outcome, settled once (C-S1)', () => {
  test('a fresh browser sign-in delivered the keys: fresh, vault unlocked, never the recovery phrase', async () => {
    const r = rig(() => ok(FRESH))
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
  })

  test('the same account signing in again is a re-sign-in with its key here, not a fresh sign-in', async () => {
    const r = rig(() => ok(SAME_ACCOUNT))
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false }])
  })

  // Task 1747 Part 1: the `done` event used to settle a fresh sign-in; it is progress now, because an outcome that is a
  // re-sign-in would have been settled as fresh by it, and the outcome that follows ignored.
  test('the done event settles nothing: only the answer does', async () => {
    let finish: (value: Answer) => void = () => {}
    const r = rig(() => new Promise((resolve) => { finish = resolve }))
    const started = r.sign.start()
    await tick()
    r.emit({ phase: 'done', email: 'sam@example.eu' })
    await tick()
    expect(r.settled).toEqual([])
    finish(ok(SAME_ACCOUNT))
    await started
    expect(r.settled).toEqual([{ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false }])
  })

  test('a late event after the answer settles nothing more: once', async () => {
    const r = rig()
    await r.sign.start()
    r.emit({ phase: 'done' })
    r.emit({ phase: 'error', message: 'late' })
    expect(r.settled).toHaveLength(1)
    expect(r.sign.state.error).toBeNull()
  })

  test('it stops listening when it settles', async () => {
    const r = rig()
    await r.sign.start()
    expect(r.unlistened()).toBe(1)
  })

  test('no timer: the controller has none to schedule (the grace of slice 2 is gone)', () => {
    expect(Object.keys(browserSignInDeps()).sort()).toEqual(['listen', 'newAttemptId', 'start'])
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
    let fail: (value: Answer) => void = () => {}
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

describe('C-S1: a different account is the password path’s outcome, and no raw code reaches a screen', () => {
  test('the switch step gets the count, the snapshot and no E3', async () => {
    const r = rig(() => ok(MISMATCH))
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 3, unsent: ASK, unsentError: null }])
  })

  test('both paths reach the switch step through one parser: the same JSON settles the same', async () => {
    for (const json of [
      MISMATCH,
      outcome({ account_mismatch: { pending_changes: 0, unsent_error: 'unsent_count_failed: no' } }),
      outcome({ account_mismatch: { pending_changes: 2, unsent: { askId: 'a', files: -1 } } }),
      outcome({ account_mismatch: { pending_changes: 1 } }),
    ]) {
      const r = rig(() => ok(json))
      await r.sign.start()
      expect(r.settled).toEqual([settledFrom(json as never)])
    }
  })

  test('a count that could not be read arrives as the coded E3 and is carried as such', async () => {
    const r = rig(() => ok(outcome({ account_mismatch: { pending_changes: 0, unsent_error: 'unsent_count_failed: no' } })))
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 0, unsent: null, unsentError: 'unsent_count_failed: no' }])
  })

  test('a snapshot that is not whole is none, never a guess', async () => {
    const r = rig(() => ok(outcome({ account_mismatch: { pending_changes: 2, unsent: { askId: 'a', files: -1 } } })))
    await r.sign.start()
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 2, unsent: null, unsentError: null }])
  })

  test('a count that is not a whole number is not a switch with an invented count: it is an error, nothing settled', async () => {
    const r = rig(() => ok(outcome({ account_mismatch: { pending_changes: 'many' } })))
    await r.sign.start()
    expect(r.settled).toEqual([])
    expect(r.sign.state.phase).toBe('error')
    expect(r.sign.state.error).toBe(SIGN_IN_OUTCOME_UNREADABLE)
  })

  test('no raw code is ever on a screen: the switch outcome shows no error, and no state mentions the code', async () => {
    const r = rig(() => ok(MISMATCH))
    await r.sign.start()
    expect(JSON.stringify(r.states)).not.toContain(['account', 'mismatch'].join('_'))
    expect(r.sign.state.error).toBeNull()
  })

  test('an account that could not be confirmed is the retryable sentence, as an error', async () => {
    const sentence = rustStr('lib.rs', 'SIGN_IN_ACCOUNT_UNKNOWN')
    const r = rig(() => ({ ok: false, reason: sentence, unsupported: false }))
    await r.sign.start()
    expect(r.sign.state).toEqual({ phase: 'error', userCode: null, verificationUri: null, error: sentence })
    expect(r.settled).toEqual([])
  })

  test('a result that is not the contract is unreadable, never a sign-in', async () => {
    for (const bad of [null, undefined, 'ok', [], {}, outcome({ account_mismatch: undefined }), outcome({ vault_unlocked: 'yes' }), outcome({ requires_2fa: true })]) {
      const r = rig(() => ({ ok: true, value: bad }))
      await r.sign.start()
      expect({ bad, settled: r.settled, error: r.sign.state.error }).toEqual({ bad, settled: [], error: SIGN_IN_OUTCOME_UNREADABLE })
    }
  })
})

// The bug this describes: Rust moves the session revision when it installs the session and emits `done` some milliseconds
// later, and AccountSessionBoundary (a one-second poll) can remount the window in between. The card that started the
// handoff is then gone, but the person IS signed in: `done` must still move the flow on (to the Finder screen), or the
// browser path, the main one, would often skip it. So a screen that goes away DETACHES: it stops being told, and the
// attempt carries on to its end (a controller of a gone screen stays listening only until then).
describe('a screen that goes away mid-handoff (a remount, or the person moved on)', () => {
  test('the answer still settles, once, and the listener is released then', async () => {
    let finish: (value: Answer) => void = () => {}
    const r = rig(() => new Promise((resolve) => { finish = resolve }))
    const started = r.sign.start()
    await tick()
    r.sign.detach()
    expect(r.unlistened()).toBe(0)
    finish(ok(FRESH))
    await started
    expect(r.settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
    expect(r.unlistened()).toBe(1)
  })

  test('a different account still reaches the switch step', async () => {
    let finish: (value: Answer) => void = () => {}
    const r = rig(() => new Promise((resolve) => { finish = resolve }))
    const started = r.sign.start()
    await tick()
    r.sign.detach()
    finish(ok(MISMATCH))
    await started
    expect(r.settled).toEqual([{ kind: 'account_mismatch', pendingChanges: 3, unsent: ASK, unsentError: null }])
  })

  test('it reports no more state to the screen that is gone', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    const reported = r.states.length
    r.sign.detach()
    r.emit({ phase: 'waiting', user_code: 'AAAA-BBBB' })
    r.emit({ phase: 'authorized' })
    expect(r.states).toHaveLength(reported)
  })

  test('an error ends it quietly: nothing settles and the listener is released', async () => {
    const r = rig(() => new Promise(() => {}))
    void r.sign.start()
    await tick()
    r.sign.detach()
    r.emit({ phase: 'error', message: 'The browser took too long.' })
    expect(r.settled).toEqual([])
    expect(r.unlistened()).toBe(1)
  })

  test('detaching before the subscription landed does not stop it from settling', async () => {
    const r = rig()
    const started = r.sign.start()
    r.sign.detach()
    await started
    expect(r.settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
    expect(r.unlistened()).toBe(1)
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
})
