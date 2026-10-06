/**
 * The frontend end of the Finder reconciler's contract (spec 2026-10-06 §9, §10): the shape it
 * serialises, the commands each action sends, and the `finder-setup-changed` subscription.
 *
 * The shape tests pin the JSON the Rust side documents (plan Task 7: `FinderSetupView`), so a
 * rename on either side fails here and not in a person's Settings window. The subscription tests
 * drive the REAL `listen` from @tauri-apps/api over a scripted `__TAURI_INTERNALS__`, so the event
 * name and the payload path are the production ones.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import {
  FINDER_ACTION_COMMAND,
  FINDER_SETUP_CHANGED_EVENT,
  loadFinderSetup,
  parseFinderSetupView,
  runFinderSetupAction,
  subscribeFinderSetup,
  type FinderSetupView,
} from '../src/finderSetup'
import type { FinderSetupAction } from '../src/finderSetupCopy'

// The sample from plan Task 7 (finder_setup/driver.rs), verbatim.
const RUST_SAMPLE = {
  setup: 'failed',
  reason: 'folder_taken',
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 1,
  last_failure: { reason: 'folder_taken', domain: 'NSCocoaErrorDomain', code: 516, at: 1791291909 },
}

const previousWindow = (globalThis as any).window
afterEach(() => { (globalThis as any).window = previousWindow })

function backend(handlers: Record<string, (args: any) => unknown>) {
  const calls: Array<{ name: string; args: any }> = []
  ;(globalThis as any).window = {
    __TAURI_INTERNALS__: {
      invoke: async (name: string, args: any) => {
        calls.push({ name, args })
        const handler = handlers[name]
        if (!handler) throw new Error(`unscripted command ${name}`)
        return handler(args)
      },
    },
  }
  return calls
}

describe('parseFinderSetupView', () => {
  test('reads exactly what the reconciler serialises', () => {
    expect(parseFinderSetupView(RUST_SAMPLE)).toEqual(RUST_SAMPLE as FinderSetupView)
  })

  test('a null reason and a null last_failure are the normal healthy shape', () => {
    const healthy = { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 4, last_failure: null }
    expect(parseFinderSetupView(healthy)).toEqual(healthy as FinderSetupView)
  })

  test('every state and every launch location of the closed vocabulary parses', () => {
    for (const setup of ['ready', 'missing', 'adding', 'failed', 'user_disabled']) {
      for (const launch_location of ['applications', 'translocated', 'disk_image', 'elsewhere']) {
        expect(parseFinderSetupView({ ...RUST_SAMPLE, setup, launch_location })).not.toBeNull()
      }
    }
  })

  test('anything outside the vocabulary is unparsable, not guessed at', () => {
    const bad: unknown[] = [
      null,
      undefined,
      'failed',
      42,
      [],
      {},
      { ...RUST_SAMPLE, setup: 'finished' },
      { ...RUST_SAMPLE, reason: 'disk_full' },
      { ...RUST_SAMPLE, launch_location: 'desktop' },
      { ...RUST_SAMPLE, attempt: '1' },
      { ...RUST_SAMPLE, max_attempts: Number.NaN },
      { ...RUST_SAMPLE, last_failure: { ...RUST_SAMPLE.last_failure, reason: 'nope' } },
      { ...RUST_SAMPLE, last_failure: { ...RUST_SAMPLE.last_failure, code: 'x' } },
      { ...RUST_SAMPLE, last_failure: 'timeout' },
    ]
    for (const value of bad) expect(parseFinderSetupView(value)).toBeNull()
  })
})

describe('loadFinderSetup', () => {
  test('asks finder_setup_state and returns the parsed view', async () => {
    const calls = backend({ finder_setup_state: () => RUST_SAMPLE })
    const result = await loadFinderSetup()
    expect(calls.map((c) => c.name)).toEqual(['finder_setup_state'])
    expect(result).toEqual({ ok: true, value: RUST_SAMPLE as FinderSetupView })
  })

  test('a rejected command is a failed result', async () => {
    backend({ finder_setup_state: () => { throw new Error('boom') } })
    const result = await loadFinderSetup()
    expect(result.ok).toBe(false)
  })

  test('an answer of the wrong shape is a failed result, not a view', async () => {
    backend({ finder_setup_state: () => ({ installed: true, path: null }) })
    const result = await loadFinderSetup()
    expect(result).toEqual({ ok: false, reason: 'Finder setup returned a state of an unexpected shape.', unsupported: false })
  })
})

describe('runFinderSetupAction sends the one command of its action', () => {
  const TABLE: Array<[FinderSetupAction, string]> = [
    ['try_again', 'finder_setup_retry'],
    ['open_system_settings', 'open_login_items_and_extensions_settings'],
    ['show_in_finder', 'finder_setup_show_app'],
    ['copy_details', 'finder_setup_copy_details'],
  ]

  test('FINDER_ACTION_COMMAND is that table', () => {
    expect(FINDER_ACTION_COMMAND).toEqual(Object.fromEntries(TABLE))
  })

  for (const [action, name] of TABLE) {
    test(`${action} -> ${name}`, async () => {
      const calls = backend({
        finder_setup_retry: () => undefined,
        open_login_items_and_extensions_settings: () => undefined,
        finder_setup_show_app: () => undefined,
        finder_setup_copy_details: () => 'details text',
      })
      const result = await runFinderSetupAction(action, { writeClipboard: async () => {} })
      expect(result.ok).toBe(true)
      expect(calls.map((c) => c.name)).toEqual([name])
    })
  }

  test('a failing command is returned, for the surface to toast', async () => {
    backend({ finder_setup_retry: () => { throw new Error('no reconciler') } })
    const result = await runFinderSetupAction('try_again')
    expect(result.ok).toBe(false)
    expect(result.ok ? '' : result.reason).toBe('no reconciler')
  })
})

/** A scripted Tauri event bus: the real `listen` registers through these two internals. */
function eventBus(opts: { rejectListen?: boolean } = {}) {
  const callbacks = new Map<number, (event: unknown) => void>()
  const log: string[] = []
  const release: Array<() => void> = []
  let nextId = 1
  let hold = false
  ;(globalThis as any).window = {
    __TAURI_INTERNALS__: {
      transformCallback: (callback: (event: unknown) => void) => {
        const id = nextId++
        callbacks.set(id, callback)
        return id
      },
      invoke: async (name: string, args: any) => {
        log.push(`${name}${args?.event ? `:${args.event}` : ''}`)
        if (name === 'plugin:event|listen') {
          if (opts.rejectListen) throw new Error('no event bus')
          if (hold) await new Promise<void>((resolve) => release.push(resolve))
          return args.handler
        }
        return undefined
      },
    },
    __TAURI_EVENT_PLUGIN_INTERNALS__: {
      unregisterListener: (event: string, id: number) => { log.push(`unregister:${event}`); callbacks.delete(id) },
    },
  }
  return {
    log,
    /** Make the next `listen` registration wait until `registered()` is called. */
    holdRegistration() { hold = true },
    registered() { hold = false; release.splice(0).forEach((resolve) => resolve()) },
    emit(payload: unknown) { for (const callback of callbacks.values()) callback({ event: FINDER_SETUP_CHANGED_EVENT, id: 0, payload }) },
    listeners: () => callbacks.size,
  }
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

describe('subscribeFinderSetup', () => {
  test('listens on finder-setup-changed and hands over the parsed view', async () => {
    const bus = eventBus()
    const seen: FinderSetupView[] = []
    const stop = subscribeFinderSetup((view) => seen.push(view))
    await settle()
    expect(FINDER_SETUP_CHANGED_EVENT).toBe('finder-setup-changed')
    expect(bus.log).toContain('plugin:event|listen:finder-setup-changed')
    bus.emit(RUST_SAMPLE)
    expect(seen).toEqual([RUST_SAMPLE as FinderSetupView])
    stop()
  })

  test('a payload of the wrong shape is reported, never handed over as a view', async () => {
    const bus = eventBus()
    const seen: unknown[] = []
    let invalid = 0
    const stop = subscribeFinderSetup((view) => seen.push(view), { onInvalid: () => { invalid += 1 } })
    await settle()
    bus.emit({ setup: 'finished' })
    bus.emit(null)
    expect(seen).toEqual([])
    expect(invalid).toBe(2)
    stop()
  })

  test('the unsubscribe stops delivery and unregisters the listener', async () => {
    const bus = eventBus()
    const seen: FinderSetupView[] = []
    const stop = subscribeFinderSetup((view) => seen.push(view))
    await settle()
    expect(bus.listeners()).toBe(1)
    stop()
    await settle()
    expect(bus.listeners()).toBe(0)
    expect(bus.log).toContain('unregister:finder-setup-changed')
    bus.emit(RUST_SAMPLE)
    expect(seen).toEqual([])
  })

  test('an unsubscribe before the registration lands still unregisters it once it does', async () => {
    const bus = eventBus()
    bus.holdRegistration()
    const stop = subscribeFinderSetup(() => {})
    stop()
    bus.registered()
    await settle()
    expect(bus.listeners()).toBe(0)
    expect(bus.log.filter((line) => line.startsWith('unregister:'))).toEqual(['unregister:finder-setup-changed'])
  })

  test('onSubscribed fires once the listener is registered, not before', async () => {
    const bus = eventBus()
    bus.holdRegistration()
    let subscribed = 0
    const stop = subscribeFinderSetup(() => {}, { onSubscribed: () => { subscribed += 1 } })
    await settle()
    expect(subscribed).toBe(0)
    bus.registered()
    await settle()
    expect(subscribed).toBe(1)
    stop()
  })

  test('onSubscribed does not fire for a subscription that was already closed', async () => {
    const bus = eventBus()
    bus.holdRegistration()
    let subscribed = 0
    const stop = subscribeFinderSetup(() => {}, { onSubscribed: () => { subscribed += 1 } })
    stop()
    bus.registered()
    await settle()
    expect(subscribed).toBe(0)
  })

  test('with no event bus it still reports ready, so the surface can load what it has', async () => {
    eventBus({ rejectListen: true })
    let subscribed = 0
    const stop = subscribeFinderSetup(() => {}, { onSubscribed: () => { subscribed += 1 } })
    await settle()
    expect(subscribed).toBe(1)
    expect(() => stop()).not.toThrow()
  })
})
