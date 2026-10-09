/**
 * `useFinderSetup` (lead ruling 7b): the load + subscribe + failed-action-toast logic that the
 * Sync tab, the Status page and the onboarding step would otherwise each copy. It is the one place
 * those three behaviours live, so they are pinned here once.
 *
 * Executes the production hook declaration with controlled hooks (tests/fixtures/componentHarness.ts)
 * against a scripted backend and a scripted event bus. What it proves: the hook's real decisions
 * for a scripted order of events. What it does NOT prove: React scheduling, or a real Tauri event
 * loop (the subscription's own wiring is pinned in tests/finderSetup.test.ts).
 *
 * Lead ruling 7a is the first group: a state that cannot be read is never "Adding Beebeeb to
 * Finder…". It is an inline unavailable state with one action that reads again.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import { commandUnavailableLabel } from '../src/desktopApi'
import * as finderSetup from '../src/finderSetup'
import type { FinderSetupController, FinderSetupOptions, FinderSetupView } from '../src/finderSetup'
import {
  FINDER_ACTION_FAILED,
  FINDER_ADDING_LINE,
  FINDER_UNAVAILABLE_LINE,
  finderSetupLoadPresentation,
} from '../src/finderSetupCopy'
import { mount, type Handler, type Mounted } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'adding',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 4,
  last_failure: null,
  ...over,
})
const failedTimeout = view({ setup: 'failed', reason: 'timeout', attempt: 4 })
const ready = view({ setup: 'ready' })

type SubscribeOptions = { onInvalid?: () => void; onSubscribed?: () => void }

/** A scripted stand-in for `subscribeFinderSetup`; `confirm()` is the registration landing. */
function fakeBus(auto: boolean) {
  const subscriptions: Array<{ onView: (v: FinderSetupView) => void; options: SubscribeOptions; closed: boolean }> = []
  const bus = {
    subscriptions,
    unsubscribes: 0,
    subscribeFinderSetup(onView: (v: FinderSetupView) => void, options: SubscribeOptions = {}) {
      const subscription = { onView, options, closed: false }
      subscriptions.push(subscription)
      if (auto) void Promise.resolve().then(() => { if (!subscription.closed) options.onSubscribed?.() })
      return () => {
        if (subscription.closed) return
        subscription.closed = true
        bus.unsubscribes += 1
      }
    },
    confirm() { for (const s of subscriptions) if (!s.closed) s.options.onSubscribed?.() },
    emit(next: FinderSetupView) { for (const s of subscriptions) if (!s.closed) s.onView(next) },
    emitInvalid() { for (const s of subscriptions) if (!s.closed) s.options.onInvalid?.() },
  }
  return bus
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

function mountHook(backend: Record<string, Handler>, opts: { auto?: boolean; deps?: FinderSetupOptions } = {}) {
  const bus = fakeBus(opts.auto ?? true)
  const m = mount('finderSetup.ts', 'useFinderSetup', {
    backend,
    expand: true,
    props: opts.deps ?? {},
    bindings: {
      loadFinderSetup: finderSetup.loadFinderSetup,
      runFinderSetupAction: finderSetup.runFinderSetupAction,
      finderSetupLoadPresentation,
      FINDER_ACTION_FAILED,
      FINDER_COPIED_MS: 2000,
      FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
      commandUnavailableLabel,
      subscribeFinderSetup: bus.subscribeFinderSetup,
      loadEngineRefusal: desktopApi.loadEngineRefusal,
    },
  })
  mounted.push(m)
  const hook = () => m.tree() as FinderSetupController
  /** Run effects, let every pending promise settle, render the result. */
  const settle = async () => { await m.flush(); await tick(); await m.flush() }
  const stateCalls = () => m.calls.filter((c) => c.name === 'finder_setup_state').length
  return { m, bus, hook, settle, stateCalls }
}

describe('lead ruling 7a: a state that cannot be read is never "Adding"', () => {
  test('a rejected finder_setup_state is the unavailable state with the existing words and ONE action', async () => {
    const h = mountHook({ finder_setup_state: () => { throw new Error('no reconciler') } })
    await h.settle()
    expect(h.hook().load).toEqual({ status: 'unavailable' })
    expect(h.hook().presentation).toEqual({ kind: 'unavailable', line: FINDER_UNAVAILABLE_LINE, actionLabel: 'Try again' })
    expect(h.hook().presentation.kind).not.toBe('adding')
  })

  test('an answer of the wrong shape is the same unavailable state', async () => {
    for (const wrong of [null, { installed: true }, { ...view(), setup: 'finished' }, 'adding']) {
      const h = mountHook({ finder_setup_state: () => wrong })
      await h.settle()
      expect(h.hook().presentation).toEqual({ kind: 'unavailable', line: FINDER_UNAVAILABLE_LINE, actionLabel: 'Try again' })
    }
  })

  test('before anything has answered it is quiet, which is not "Adding" either', async () => {
    const h = mountHook({ finder_setup_state: () => new Promise(() => {}) })
    expect(h.hook().presentation).toEqual({ kind: 'quiet', line: '' })
    await h.settle()
    expect(h.hook().load).toEqual({ status: 'loading' })
    expect(h.hook().presentation).toEqual({ kind: 'quiet', line: '' })
  })

  test('retry reads again: back to quiet while it asks, then the real state when it answers', async () => {
    let answers = 0
    let gate!: () => void
    const h = mountHook({
      finder_setup_state: async () => {
        answers += 1
        if (answers === 1) throw new Error('not yet')
        await new Promise<void>((resolve) => { gate = resolve })
        return ready
      },
    })
    await h.settle()
    expect(h.hook().presentation.kind).toBe('unavailable')

    const retrying = h.hook().retry()
    h.m.render()
    expect(h.hook().presentation).toEqual({ kind: 'quiet', line: '' })
    gate()
    await retrying
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
    expect(h.stateCalls()).toBe(2)
  })

  // M1 (triage 19): a re-read from a view that IS there keeps it on screen while it asks, so "Repair…"
  // and the row do not vanish for one round trip after every repair. Only `unavailable` blanks.
  test('retry from a loaded view keeps that view on screen while it reads', async () => {
    let answers = 0
    let gate!: () => void
    const h = mountHook({
      finder_setup_state: async () => {
        answers += 1
        if (answers === 1) return ready
        await new Promise<void>((resolve) => { gate = resolve })
        return failedTimeout
      },
    })
    await h.settle()
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
    const retrying = h.hook().retry()
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
    gate()
    await retrying
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: failedTimeout })
  })

  // M1 (triage 17): two reads in flight; only the newest issued may land.
  test('a read that answers after a newer read was issued is dropped', async () => {
    const answers: Array<(v: FinderSetupView) => void> = []
    const h = mountHook({ finder_setup_state: () => new Promise<FinderSetupView>((resolve) => { answers.push(resolve) }) })
    await h.settle()
    expect(answers).toHaveLength(1)
    const retrying = h.hook().retry()
    await tick()
    expect(answers).toHaveLength(2)
    answers[1](ready)
    await retrying
    answers[0](view()) // the first read answers last: it is older, so it is dropped
    await tick()
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
  })

  test('retry that fails again lands on unavailable again, still with its one action', async () => {
    const h = mountHook({ finder_setup_state: () => { throw new Error('still no') } })
    await h.settle()
    await h.hook().retry()
    h.m.render()
    expect(h.hook().presentation).toEqual({ kind: 'unavailable', line: FINDER_UNAVAILABLE_LINE, actionLabel: 'Try again' })
    expect(h.stateCalls()).toBe(2)
  })
})

describe('lead ruling 7b: load + subscribe', () => {
  test('the first view is the one finder_setup_state answers', async () => {
    const h = mountHook({ finder_setup_state: () => view() })
    await h.settle()
    expect(h.hook().load).toEqual({ status: 'loaded', view: view() })
    expect(h.hook().presentation).toEqual({ kind: 'adding', line: FINDER_ADDING_LINE })
  })

  test('the initial load starts only after the subscription has landed, so no transition falls between the two', async () => {
    const h = mountHook({ finder_setup_state: () => view() }, { auto: false })
    await h.settle()
    expect(h.bus.subscriptions.length).toBe(1)
    expect(h.stateCalls()).toBe(0)
    h.bus.confirm()
    await h.settle()
    expect(h.stateCalls()).toBe(1)
    expect(h.hook().load.status).toBe('loaded')
  })

  test('re-rendering does not subscribe or load again', async () => {
    const h = mountHook({ finder_setup_state: () => view() })
    await h.settle()
    for (let i = 0; i < 5; i++) await h.m.flush()
    expect(h.bus.subscriptions.length).toBe(1)
    expect(h.stateCalls()).toBe(1)
  })

  test('every event replaces the view: adding, then a failure, then ready', async () => {
    const h = mountHook({ finder_setup_state: () => view() })
    await h.settle()
    h.bus.emit(failedTimeout)
    h.m.render()
    expect(h.hook().presentation).toMatchObject({ kind: 'notice', reason: 'timeout', actionLabel: 'Try again' })
    h.bus.emit(ready)
    h.m.render()
    expect(h.hook().presentation.kind).toBe('ready')
    expect(h.stateCalls()).toBe(1)
  })

  test('an event is a view even when the first read had failed', async () => {
    const h = mountHook({ finder_setup_state: () => { throw new Error('early') } })
    await h.settle()
    expect(h.hook().load.status).toBe('unavailable')
    h.bus.emit(ready)
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
  })

  test('an event that arrives while a read is in flight wins over that read', async () => {
    let answer!: (v: FinderSetupView) => void
    const h = mountHook({ finder_setup_state: () => new Promise<FinderSetupView>((resolve) => { answer = resolve }) })
    await h.m.flush()
    await tick()
    expect(h.stateCalls()).toBe(1)
    h.bus.emit(failedTimeout)
    answer(view()) // the older read finishes after the newer event
    await tick()
    await h.m.flush()
    expect(h.hook().load).toEqual({ status: 'loaded', view: failedTimeout })
  })

  test('an event payload the parser rejected triggers a fresh read instead of a guess', async () => {
    let current: FinderSetupView = view()
    const h = mountHook({ finder_setup_state: () => current })
    await h.settle()
    current = ready
    h.bus.emitInvalid()
    await h.settle()
    expect(h.stateCalls()).toBe(2)
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
  })

  test('unmounting unsubscribes exactly once', async () => {
    const h = mountHook({ finder_setup_state: () => view() })
    await h.settle()
    expect(h.bus.unsubscribes).toBe(0)
    h.m.unmount()
    expect(h.bus.unsubscribes).toBe(1)
    h.m.unmount()
    expect(h.bus.unsubscribes).toBe(1)
  })

  test('a read that finishes after unmount changes nothing', async () => {
    let answer!: (v: FinderSetupView) => void
    const h = mountHook({ finder_setup_state: () => new Promise<FinderSetupView>((resolve) => { answer = resolve }) })
    await h.m.flush()
    await tick()
    h.m.unmount()
    answer(ready)
    await tick()
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loading' })
  })

  test('an event that lands after unmount changes nothing', async () => {
    const h = mountHook({ finder_setup_state: () => view() })
    await h.settle()
    h.m.unmount()
    h.bus.emit(ready)
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: view() })
  })
})

describe('lead ruling 7b: actions and the failed-action toast', () => {
  const backendWith = (over: Record<string, Handler> = {}): Record<string, Handler> => ({
    finder_setup_state: () => failedTimeout,
    finder_setup_retry: () => undefined,
    open_login_items_and_extensions_settings: () => undefined,
    finder_setup_show_app: () => undefined,
    finder_setup_copy_details: () => 'details text',
    ...over,
  })

  test('an action that works sends its command and raises no toast', async () => {
    const h = mountHook(backendWith())
    await h.settle()
    const result = await h.hook().run('try_again')
    expect(result.ok).toBe(true)
    expect(h.m.calls.map((c) => c.name)).toContain('finder_setup_retry')
    expect(h.m.toasts).toEqual([])
  })

  // Task 17b (lead ruling): with every macOS FpError redacted to a domain and a code, a failed action
  // never renders `result.reason`. It is one error toast with one fixed sentence, and no title.
  const SENTENCES = {
    try_again: 'Beebeeb couldn’t retry adding itself to Finder.',
    open_system_settings: 'Beebeeb couldn’t open System Settings.',
    show_in_finder: 'Beebeeb couldn’t show itself in Finder.',
    copy_details: 'Beebeeb couldn’t copy the details.',
  } as const

  test('a failed action is one error toast with the action\'s one sentence, and the reason is not shown', async () => {
    const h = mountHook(backendWith({ finder_setup_show_app: () => { throw new Error('io.beebeeb.bridge 3') } }))
    await h.settle()
    const result = await h.hook().run('show_in_finder')
    expect(result.ok).toBe(false)
    expect(h.m.toasts).toEqual([{ variant: 'error', message: SENTENCES.show_in_finder }])
    expect(JSON.stringify(h.m.toasts)).not.toContain('io.beebeeb')
  })

  // Try again is the exception (row 15, below): Rust's finder_setup_retry fails only with fixed
  // sentences, so its failure is said verbatim. The other three keep one fixed sentence each.
  test('every other action has its own sentence, and the bridge code appears in none of them', async () => {
    const throwing = () => { throw new Error('io.beebeeb.bridge 3') }
    const h = mountHook(backendWith({
      open_login_items_and_extensions_settings: throwing,
      finder_setup_show_app: throwing,
      finder_setup_copy_details: throwing,
    }))
    await h.settle()
    for (const action of ['open_system_settings', 'show_in_finder', 'copy_details'] as const) {
      await h.hook().run(action)
    }
    expect(h.m.toasts.map((t) => t.message)).toEqual([
      SENTENCES.open_system_settings,
      SENTENCES.show_in_finder,
      SENTENCES.copy_details,
    ])
    expect(h.m.toasts.map((t) => t.variant)).toEqual(['error', 'error', 'error'])
    expect(h.m.toasts.some((t) => 'title' in t && t.title !== undefined)).toBe(false)
    expect(JSON.stringify(h.m.toasts)).not.toContain('io.beebeeb')
  })

  test('a command this build does not have gets the same one sentence: no command name, no raw error', async () => {
    const h = mountHook(backendWith({ finder_setup_show_app: () => { throw new Error('finder_setup_show_app is not a registered command') } }))
    await h.settle()
    await h.hook().run('show_in_finder')
    expect(h.m.toasts).toEqual([{ variant: 'error', message: SENTENCES.show_in_finder }])
  })

  // FT-clipboard: the write starts inside the click (WebKit's transient activation), and a success says
  // "Copied" on the button for a moment.
  test('copy_details: the write starts synchronously inside run, before any await', async () => {
    let writes = 0
    const h = mountHook(backendWith(), { deps: { writeClipboard: async (text) => { writes += 1; await text } } })
    await h.settle()
    const pending = h.hook().run('copy_details')
    expect(writes).toBe(1)
    await pending
  })

  test('copy_details: a success sets `copied` for a moment; any other action, or a failure, does not', async () => {
    const h = mountHook(backendWith(), { deps: { writeClipboard: async (text) => { await text } } })
    await h.settle()
    expect(h.hook().copied).toBe(false)
    await h.hook().run('copy_details')
    h.m.render()
    expect(h.hook().copied).toBe(true)
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().copied).toBe(false)
  })

  test('copy_details: a refused pasteboard does not say Copied', async () => {
    const h = mountHook(backendWith(), { deps: { writeClipboard: async () => { throw new Error('denied') } } })
    await h.settle()
    await h.hook().run('copy_details')
    h.m.render()
    expect(h.hook().copied).toBe(false)
  })

  // 17b-M3: a failed action's reason is not shown, but it leaves a trace for diagnostics: the action and
  // a reason CODE only (a redacted domain and code, or a fixed word), never free text.
  test('a failed action leaves a console trace of the action and a reason code, never free text', async () => {
    const traced: unknown[][] = []
    const original = console.warn
    console.warn = (...args: unknown[]) => { traced.push(args) }
    try {
      const reasons: Record<string, () => never> = {
        open_login_items_and_extensions_settings: () => { throw new Error('io.beebeeb.bridge 3') },
        finder_setup_show_app: () => { throw new Error('could not reveal /Users/sam/Applications/Beebeeb.app') },
        finder_setup_copy_details: () => { throw new Error('finder_setup_copy_details is not a registered command') },
      }
      const h = mountHook(backendWith(reasons))
      await h.settle()
      await h.hook().run('open_system_settings')
      await h.hook().run('show_in_finder')
      await h.hook().run('copy_details')
      expect(traced).toEqual([
        ['open_system_settings', 'io.beebeeb.bridge 3'],
        ['show_in_finder', 'other'],
        ['copy_details', 'unsupported'],
      ])
      expect(JSON.stringify(traced)).not.toContain('/Users')
    } finally {
      console.warn = original
    }
  })

  test('copy_details: the pasteboard refusing is a toast, and the details were still asked for', async () => {
    const h = mountHook(backendWith(), { deps: { writeClipboard: async () => { throw new Error('denied') } } })
    await h.settle()
    const result = await h.hook().run('copy_details')
    expect(result.ok).toBe(false)
    expect(h.m.calls.map((c) => c.name)).toContain('finder_setup_copy_details')
    expect(h.m.toasts).toEqual([{ variant: 'error', message: SENTENCES.copy_details }])
  })

  test('copy_details: the text the command returned reaches the pasteboard, with no toast', async () => {
    const written: string[] = []
    const h = mountHook(backendWith(), { deps: { writeClipboard: async (text) => { written.push(await text) } } })
    await h.settle()
    const result = await h.hook().run('copy_details')
    expect(result.ok).toBe(true)
    expect(written).toEqual(['details text'])
    expect(h.m.toasts).toEqual([])
  })

  test('a failed action never replaces the view the surface is showing', async () => {
    const h = mountHook(backendWith({ finder_setup_retry: () => { throw rustStr('lib.rs', 'FINDER_SETUP_HELD') } }))
    await h.settle()
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().load).toEqual({ status: 'loaded', view: failedTimeout })
    expect(h.hook().presentation).toMatchObject({ kind: 'notice', reason: 'timeout' })
  })
})

/**
 * Must-render row 15 (M3, FA-M4): `finder_setup_retry` fails with one of two fixed Rust sentences, the
 * reconciler not running (NOT_RUNNING, "…Quit and reopen Beebeeb to start it again.") and the reconciler
 * held (FINDER_SETUP_HELD, the quiet-Missing sentence). Both carry the remedy, so the hook shows the
 * sentence verbatim as a neutral note (`actionNote`), never under "Couldn’t retry" and never as a
 * fixed toast that hides it. The note goes when the state changes or the next action runs.
 */
describe('a failed Try again says Rust\'s sentence (row 15)', () => {
  const HELD = rustStr('lib.rs', 'FINDER_SETUP_HELD')
  const NOT_RUNNING = rustStr('finder_setup/driver.rs', 'NOT_RUNNING')
  const failing = (reason: unknown) => ({ finder_setup_state: () => failedTimeout, finder_setup_retry: () => { throw reason } })

  for (const [name, sentence] of [['held', HELD], ['not running', NOT_RUNNING]] as const) {
    test(`${name}: the sentence verbatim as a neutral note, and no toast`, async () => {
      const h = mountHook(failing(sentence))
      await h.settle()
      const result = await h.hook().run('try_again')
      h.m.render()
      expect(result.ok).toBe(false)
      expect(h.hook().actionNote).toBe(sentence)
      expect(h.m.toasts).toEqual([])
    })
  }

  test('a failure that is not one of Rust\'s answers (the command is missing) keeps the fixed toast', async () => {
    const h = mountHook(failing(new Error('finder_setup_retry is not a registered command')))
    await h.settle()
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().actionNote).toBeNull()
    expect(h.m.toasts).toEqual([{ variant: 'error', message: 'Beebeeb couldn’t retry adding itself to Finder.' }])
  })

  test('the note goes on the next finder-setup-changed, and when the next action runs', async () => {
    let fail = true
    const h = mountHook({ finder_setup_state: () => failedTimeout, finder_setup_retry: () => { if (fail) throw HELD } })
    await h.settle()
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().actionNote).toBe(HELD)
    h.bus.emit(view())
    h.m.render()
    expect(h.hook().actionNote).toBeNull()
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().actionNote).toBe(HELD)
    fail = false
    await h.hook().run('try_again')
    h.m.render()
    expect(h.hook().actionNote).toBeNull()
  })
})

/**
 * Must-render row 9 (FT-I5): there is no event for an engine refusal, so the hook reads
 * `sync_status.engine_refusal` when it loads and again on every `finder-setup-changed`, and a
 * `failed` + `unknown` presents the refusal's sentence.
 */
describe('the engine refusal (row 9)', () => {
  const IDENTITY = { code: 'identity_unknown', sentence: 'Beebeeb couldn’t confirm which account this computer’s local files belong to, so sync didn’t start. Connect to the internet and open Beebeeb again.' }
  const status = (engine_refusal: unknown) => ({ logged_in: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal })
  const unknownFailure = view({ setup: 'failed', reason: 'unknown' })
  const statusCalls = (h: ReturnType<typeof mountHook>) => h.m.calls.filter((c) => c.name === 'sync_status').length

  test('it reads sync_status on load, and a failed + unknown says the refusal\'s sentence', async () => {
    const h = mountHook({ finder_setup_state: () => unknownFailure, sync_status: () => status(IDENTITY) })
    await h.settle()
    expect(statusCalls(h)).toBe(1)
    expect(h.hook().presentation).toMatchObject({ kind: 'notice', sentence: IDENTITY.sentence, action: 'try_again' })
  })

  test('it reads sync_status again on every finder-setup-changed, and follows what it says', async () => {
    let refusal: unknown = null
    const h = mountHook({ finder_setup_state: () => view(), sync_status: () => status(refusal) })
    await h.settle()
    expect(statusCalls(h)).toBe(1)
    refusal = IDENTITY
    h.bus.emit(unknownFailure)
    await h.settle()
    expect(statusCalls(h)).toBe(2)
    expect(h.hook().presentation).toMatchObject({ sentence: IDENTITY.sentence })
    refusal = null
    h.bus.emit(unknownFailure)
    await h.settle()
    expect(statusCalls(h)).toBe(3)
    expect(h.hook().presentation).toMatchObject({ sentence: 'Beebeeb couldn’t be added to Finder.' })
  })

  test('a refusal read that answers after a newer one is dropped', async () => {
    const answers: Array<(v: unknown) => void> = []
    const h = mountHook({ finder_setup_state: () => unknownFailure, sync_status: () => new Promise((resolve) => { answers.push(resolve) }) })
    await h.settle()
    h.bus.emit(unknownFailure)
    await h.settle()
    expect(answers).toHaveLength(2)
    answers[1](status(null))
    await h.settle()
    answers[0](status(IDENTITY)) // the older read finishes last
    await h.settle()
    expect(h.hook().presentation).toMatchObject({ sentence: 'Beebeeb couldn’t be added to Finder.' })
  })

  test('a status that cannot be read leaves the generic sentence', async () => {
    const h = mountHook({ finder_setup_state: () => unknownFailure, sync_status: () => { throw new Error('down') } })
    await h.settle()
    expect(h.hook().presentation).toMatchObject({ sentence: 'Beebeeb couldn’t be added to Finder.', action: 'try_again' })
  })

  test('a disabled hook never reads sync_status', async () => {
    const h = mountHook({ finder_setup_state: () => unknownFailure, sync_status: () => status(IDENTITY) }, { deps: { enabled: false } })
    await h.settle()
    expect(statusCalls(h)).toBe(0)
  })
})

/**
 * Task 17. SyncFolder and Status also run on Windows and Linux, and a hook cannot be called
 * conditionally, so they call it always and say whether this host is a Mac. While it is not
 * enabled the hook reads nothing, listens to nothing and presents nothing: on those hosts the
 * reconciler's commands and event are never touched.
 */
describe('enabled: a surface that also exists off macOS', () => {
  const backend = (): Record<string, Handler> => ({ finder_setup_state: () => ready, finder_setup_retry: () => undefined })

  test('disabled: no read, no subscription, nothing to present', async () => {
    const h = mountHook(backend(), { deps: { enabled: false } })
    await h.settle()
    await h.settle()
    expect(h.stateCalls()).toBe(0)
    expect(h.bus.subscriptions).toHaveLength(0)
    expect(h.hook().load).toEqual({ status: 'loading' })
    expect(h.hook().presentation).toEqual({ kind: 'quiet', line: '' })
  })

  test('disabled: retry reads nothing either', async () => {
    const h = mountHook(backend(), { deps: { enabled: false } })
    await h.settle()
    await h.hook().retry()
    await h.settle()
    expect(h.stateCalls()).toBe(0)
    expect(h.hook().load).toEqual({ status: 'loading' })
  })

  test('enabled later (the platform resolves after the first render): it subscribes and reads once', async () => {
    const deps: FinderSetupOptions = { enabled: false }
    const h = mountHook(backend(), { deps })
    await h.settle()
    expect(h.bus.subscriptions).toHaveLength(0)
    deps.enabled = true
    h.m.render()
    await h.settle()
    expect(h.bus.subscriptions).toHaveLength(1)
    expect(h.stateCalls()).toBe(1)
    expect(h.hook().load).toEqual({ status: 'loaded', view: ready })
    for (let i = 0; i < 4; i++) await h.m.flush()
    expect(h.stateCalls()).toBe(1)
  })

  test('not passing it is enabled, as every existing caller expects', async () => {
    const h = mountHook(backend())
    await h.settle()
    expect(h.bus.subscriptions).toHaveLength(1)
    expect(h.stateCalls()).toBe(1)
  })
})
