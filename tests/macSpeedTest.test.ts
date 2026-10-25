/**
 * The speed test's store (task 1683 slice 5; PR #140 review, P2-7 and P3): one test at a time for the whole window, a result
 * that is a figure or a plain failure and never "0.00 Mbps", and a state that outlives the page that started it.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createSpeedTest } from '../src/macSpeedTest'
import { SPEED_TEST_NO_FIGURES, speedTestFailure } from '../src/macSettingsModel'

const RESULT = { latency: { min_ms: 18, avg_ms: 24, max_ms: 41 }, upload_bps: 4_400_000, download_bps: 11_750_000 }

describe('the result a person is shown', () => {
  test('a direction that moved nothing, or no number at all, is a failure with the plain sentence', () => {
    expect(speedTestFailure(RESULT)).toBeNull()
    expect(speedTestFailure({ ...RESULT, upload_bps: 0 })).toBe(SPEED_TEST_NO_FIGURES)
    expect(speedTestFailure({ ...RESULT, download_bps: 0 })).toBe(SPEED_TEST_NO_FIGURES)
    expect(speedTestFailure({ ...RESULT, upload_bps: Number.NaN })).toBe(SPEED_TEST_NO_FIGURES)
    expect(speedTestFailure({ ...RESULT, download_bps: -1 })).toBe(SPEED_TEST_NO_FIGURES)
  })

  test('a latency of 0 ms is a real figure on a fast link: it never fails a run', () => {
    expect(speedTestFailure({ ...RESULT, latency: { min_ms: 0, avg_ms: 0, max_ms: 0 } })).toBeNull()
  })

  test('the sentence says what to do and shows no developer word', () => {
    expect(SPEED_TEST_NO_FIGURES).toBe('The test finished without a speed to report. Check your internet connection and try again.')
  })
})

describe('the store', () => {
  const ok = (value = RESULT) => Promise.resolve({ ok: true as const, value })

  test('starts idle, is running while the host works, and keeps the figures when it is done', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const store = createSpeedTest(async () => { await gate; return ok() })
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
    const settled = store.start()
    expect(store.getSnapshot()).toEqual({ phase: 'running' })
    release()
    expect(await settled).toBeNull()
    expect(store.getSnapshot()).toEqual({ phase: 'done', result: RESULT })
  })

  test('a second start while one runs does nothing: one run, no message', async () => {
    let runs = 0
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const store = createSpeedTest(async () => { runs += 1; await gate; return ok() })
    const first = store.start()
    expect(await store.start()).toBeNull()
    expect(runs).toBe(1)
    release()
    await first
    expect(runs).toBe(1)
  })

  test('a test can be run again once it has ended, and the new figures replace the old', async () => {
    let n = 0
    const store = createSpeedTest(() => ok({ ...RESULT, download_bps: 1_000_000 * ++n }))
    await store.start()
    await store.start()
    expect(store.getSnapshot()).toEqual({ phase: 'done', result: { ...RESULT, download_bps: 2_000_000 } })
  })

  test('a failure goes back to idle with the host\'s own reason, and the next press can try again', async () => {
    let n = 0
    const store = createSpeedTest(async () => (++n === 1 ? { ok: false as const, reason: 'None of the 3 uploads finished. Check your internet connection and try again.' } : ok()))
    expect(await store.start()).toBe('None of the 3 uploads finished. Check your internet connection and try again.')
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
    expect(await store.start()).toBeNull()
    expect(store.getSnapshot().phase).toBe('done')
  })

  test('a host without the command says so in one sentence, not a raw error', async () => {
    const store = createSpeedTest(async () => ({ ok: false as const, reason: 'command run_speedtest not found', unsupported: true }))
    const reason = await store.start()
    expect(reason).toBeString()
    expect(reason).not.toContain('not found')
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
  })

  test('a result without a speed is a failure, not figures', async () => {
    const store = createSpeedTest(() => ok({ ...RESULT, upload_bps: 0, download_bps: 0 }))
    expect(await store.start()).toBe(SPEED_TEST_NO_FIGURES)
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
  })

  test('listeners hear every change, and none after they leave', async () => {
    const store = createSpeedTest(() => ok())
    const heard: string[] = []
    const leave = store.subscribe(() => heard.push(store.getSnapshot().phase))
    await store.start()
    leave()
    await store.start()
    expect(heard).toEqual(['running', 'done'])
  })
})

// PR #140 delta review, P3: the figures belong to the account and the connection they were measured on. The store lives as
// long as the window does, so a session that ends (a sign-out, another account, a lock that ends the session) clears it.
describe('a session change clears the store', () => {
  /** A stand-in for `subscribeAccountSession`: `change()` is the session revision moving. */
  function session() {
    const listeners = new Set<() => void>()
    return { subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }, change: () => listeners.forEach((l) => l()) }
  }
  const ok = (value = RESULT) => Promise.resolve({ ok: true as const, value })

  test('the figures of a finished test are gone when the session changes', async () => {
    const s = session()
    const store = createSpeedTest(() => ok(), s.subscribe)
    await store.start()
    expect(store.getSnapshot().phase).toBe('done')
    s.change()
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
  })

  test('a test that was running when the session changed is dropped: its figures are not shown, it asks for no toast', async () => {
    const s = session()
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const store = createSpeedTest(async () => { await gate; return ok() }, s.subscribe)
    const running = store.start()
    expect(store.getSnapshot().phase).toBe('running')
    s.change()
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
    release()
    expect(await running).toBeNull()
    expect(store.getSnapshot()).toEqual({ phase: 'idle' })
  })

  test('a failure that arrives after the session changed is not a message for the new session either', async () => {
    const s = session()
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const store = createSpeedTest(async () => { await gate; return { ok: false as const, reason: 'offline' } }, s.subscribe)
    const running = store.start()
    s.change()
    release()
    expect(await running).toBeNull()
  })

  test('a test started after the change is the new session\'s and answers normally', async () => {
    const s = session()
    const store = createSpeedTest(() => ok(), s.subscribe)
    await store.start()
    s.change()
    expect(await store.start()).toBeNull()
    expect(store.getSnapshot()).toEqual({ phase: 'done', result: RESULT })
  })

  test('a change while nothing was measured tells nobody anything', async () => {
    const s = session()
    const store = createSpeedTest(() => ok(), s.subscribe)
    let heard = 0
    store.subscribe(() => { heard += 1 })
    s.change()
    expect(heard).toBe(0)
  })

  test('the window\'s own store is wired to the account session, the one the whole window observes', () => {
    const source = readFileSync(new URL('../src/macSpeedTest.ts', import.meta.url), 'utf8').replace(/\r\n/g, '\n')
    expect(source).toContain('export const desktopSpeedTest = createSpeedTest(runSpeedtest, subscribeAccountSession)')
  })
})
