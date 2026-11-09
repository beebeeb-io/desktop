// The speed test reports the command's own answer. Figures appear only after `run_speedtest` returns,
// in Mbps from its bytes per second. A refusal is one of the existing sentences, never a code.
// Called by `bun test tests/speedtestHonest.test.ts` (slice 27). No production module imports it.
import { afterAll, afterEach, beforeAll, describe, expect, test } from 'bun:test'
import { accountSessionRevision, observeAccountSession } from '../src/accountSession'
import { clearSpeedTest, failureSentence, runSpeedTest, testStateFrom, useSpeedTest, type TestState } from '../src/v2/sections/settings/today/speedtest'
import { SpeedTest } from '../src/v2/sections/settings/today/bandwidth'
import { allElements, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const ANSWER = { latency: { min_ms: 18, avg_ms: 24, max_ms: 41 }, upload_bps: 4_400_000, download_bps: 11_750_000 }
/** Rust's own sentence, curly apostrophe included (`SPEEDTEST_NO_PING`). */
const PING_REFUSAL = 'Beebeeb didn’t answer any of the 5 pings. Check your internet connection and try again.'
const UPLOAD_REFUSAL = 'None of the 3 uploads finished. Check your internet connection and try again.'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let calls: string[] = []
let answer: () => Promise<unknown> = () => Promise.reject(new Error('unscripted run_speedtest'))

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

function scriptInvoke() {
  dom.window.__TAURI_INTERNALS__ = {
    invoke(name: string) {
      calls.push(name)
      if (name !== 'run_speedtest') return Promise.reject(new Error(`unscripted ${name}`))
      return answer()
    },
  }
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)) })
}

function mount(node: import('react').ReactElement) {
  const container = dom.document.createElement('div') as MiniElement
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  return {
    container,
    root,
    show: async (next: import('react').ReactElement) => { await React.act(async () => { root.render(next) }); await settle() },
  }
}

function Probe() {
  const state = useSpeedTest()
  return React.createElement('pre', { 'data-testid': 'snap' }, JSON.stringify(state))
}

async function read(): Promise<TestState> {
  const view = mount(React.createElement(Probe))
  await view.show(React.createElement(Probe))
  const text = view.container.textContent
  await React.act(async () => view.root.unmount())
  return JSON.parse(text) as TestState
}

function buttons(container: MiniElement): string[] {
  return allElements(container).filter((el) => el.tagName === 'BUTTON').map((el) => el.textContent.trim())
}

describe.serial('the speed test reports what it measured', () => {
  afterEach(() => {
    clearSpeedTest()
    calls = []
    answer = () => Promise.reject(new Error('unscripted run_speedtest'))
    scriptInvoke()
  })

  test('no figure until the command answers, then the Mbps it measured, and the step is named', async () => {
    let release!: (value: unknown) => void
    const gate = new Promise((resolve) => { release = resolve })
    answer = () => gate.then(() => ANSWER)
    scriptInvoke()
    clearSpeedTest()
    const page = mount(React.createElement(SpeedTest))
    runSpeedTest()
    await page.show(React.createElement(SpeedTest))
    const running = await read()
    expect(running.phase).toBe('running')
    if (running.phase === 'running') {
      expect(running.step).toBe('latency')
      expect(running.up).toBeUndefined()
      expect(page.container.textContent).toContain('Latency')
      expect(page.container.textContent).toContain('Pinging')
    }
    expect(page.container.textContent).not.toContain('35.2')
    expect(page.container.textContent).not.toContain('94.0')
    expect(page.container.textContent).not.toContain('4.4')
    expect(calls).toEqual(['run_speedtest'])

    await React.act(async () => { release(undefined) })
    await settle()
    await page.show(React.createElement(SpeedTest))
    const done = testStateFrom({ ok: true, value: ANSWER })
    // Bytes per second → megabits per second: × 8 / 1_000_000. 4_400_000 B/s is 35.2 Mbps, not 4.4.
    expect(done).toEqual({ phase: 'done', result: { latencyAvg: 24, latencyMin: 18, latencyMax: 41, up: 35.2, down: 94 } })
    expect(await read()).toEqual(done)
    expect(page.container.textContent).toContain('24 ms')
    expect(page.container.textContent).toContain('18 to 41 ms')
    expect(page.container.textContent).toContain('35.2 Mbps')
    expect(page.container.textContent).toContain('94.0 Mbps')
    expect(page.container.textContent).not.toContain('4.4')
    expect(page.container.textContent).not.toContain('11.8')
    await React.act(async () => page.root.unmount())
  })

  test('a refusal is the existing sentence, with no code and no invented 0 Mbps', () => {
    expect(testStateFrom({ ok: false, reason: PING_REFUSAL })).toEqual({ phase: 'error', message: failureSentence('ping') })
    expect(testStateFrom({ ok: false, reason: UPLOAD_REFUSAL })).toEqual({ phase: 'error', message: failureSentence('upload') })
    expect(testStateFrom({ ok: false, reason: 'None of the 3 downloads finished. Check your internet connection and try again.' })).toEqual({
      phase: 'error', message: failureSentence('download'),
    })
    const coded = testStateFrom({ ok: false, reason: `SPEEDTEST_NO_PING ${PING_REFUSAL}` })
    expect(coded.phase).toBe('error')
    if (coded.phase === 'error') {
      expect(coded.message).toBe(failureSentence('ping'))
      expect(coded.message).not.toContain('SPEEDTEST_NO_PING')
      expect(coded.message).not.toMatch(/\b[A-Z]{2,}_[A-Z0-9_]+\b/)
    }
    const empty = testStateFrom({ ok: true, value: { ...ANSWER, upload_bps: 0, download_bps: 0 } })
    expect(empty).toEqual({ phase: 'error', message: failureSentence('upload') })
    expect(JSON.stringify(empty)).not.toContain('0.0')
    expect(JSON.stringify(empty)).not.toContain('Mbps')
    const none = testStateFrom({ ok: false, reason: 'SPEEDTEST_NO_UPLOAD' })
    expect(none.phase).toBe('error')
    if (none.phase === 'error') expect(none.message).not.toContain('SPEEDTEST')
  })

  test('a second click while running does nothing and says so, and a finished result lasts until sign-out or lock', async () => {
    let release!: (value: unknown) => void
    const gate = new Promise((resolve) => { release = resolve })
    answer = () => gate.then(() => ANSWER)
    scriptInvoke()
    clearSpeedTest()
    const page = mount(React.createElement(SpeedTest))
    runSpeedTest()
    const again = runSpeedTest()
    await page.show(React.createElement(SpeedTest))
    expect(calls).toEqual(['run_speedtest'])
    expect(again).toBe('A speed test is already running.')
    expect(page.container.textContent).toContain('A speed test is already running.')
    const mid = await read()
    expect(mid.phase).toBe('running')

    await React.act(async () => { release(undefined) })
    await settle()
    await page.show(React.createElement(SpeedTest))
    const finished = await read()
    expect(finished.phase).toBe('done')
    await React.act(async () => page.root.unmount())
    const returned = mount(React.createElement(SpeedTest))
    await returned.show(React.createElement(SpeedTest))
    expect(returned.container.textContent).toContain('35.2 Mbps')
    expect(await read()).toEqual(finished)

    await React.act(async () => { observeAccountSession((accountSessionRevision() ?? 0) + 1) })
    await returned.show(React.createElement(SpeedTest))
    expect((await read()).phase).toBe('idle')
    expect(returned.container.textContent).not.toContain('35.2')

    let releaseLate!: (value: unknown) => void
    const late = new Promise((resolve) => { releaseLate = resolve })
    answer = () => late.then(() => ANSWER)
    calls = []
    await React.act(async () => { runSpeedTest() })
    expect((await read()).phase).toBe('running')
    await React.act(async () => { observeAccountSession((accountSessionRevision() ?? 0) + 1) })
    expect((await read()).phase).toBe('running')
    let said: string | null = null
    await React.act(async () => { said = runSpeedTest() })
    expect(said).toBe('A speed test is already running.')
    expect(calls).toEqual(['run_speedtest'])
    await React.act(async () => { releaseLate(undefined) })
    await settle()
    expect((await read()).phase).toBe('idle')
    expect(JSON.stringify(await read())).not.toContain('35.2')
    await React.act(async () => returned.root.unmount())
  })

  test('a build without run_speedtest says so and offers no button, and a plain browser walks no board steps', async () => {
    answer = () => Promise.reject(new Error('unknown command run_speedtest'))
    scriptInvoke()
    clearSpeedTest()
    runSpeedTest()
    await settle()
    const missing = await read()
    expect(missing.phase).toBe('unavailable')
    if (missing.phase === 'unavailable') {
      expect(missing.message).toContain('not available in this build')
      expect(missing.message).not.toMatch(/run_speedtest|not found|unknown command|SPEEDTEST/i)
    }
    const page = mount(React.createElement(SpeedTest))
    await page.show(React.createElement(SpeedTest))
    expect(page.container.textContent).toContain('not available in this build')
    expect(buttons(page.container)).not.toContain('Run test')
    expect(buttons(page.container)).not.toContain('Run test again')
    expect(buttons(page.container)).not.toContain('Running…')
    await React.act(async () => page.root.unmount())

    const previousDev = import.meta.env.DEV
    const planted = { phase: 'done', result: { latencyAvg: 1, latencyMin: 1, latencyMax: 1, up: 999, down: 999 } }
    try {
      ;(import.meta.env as { DEV?: unknown }).DEV = '1'
      ;(globalThis as Record<string, unknown>).__bbV2Board = { speedtestSteps: [planted] }
      delete dom.window.__TAURI_INTERNALS__
      clearSpeedTest()
      calls = []
      runSpeedTest()
      await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 1200)) })
      const walked = JSON.stringify(await read())
      expect(walked).not.toContain('999')
      expect(calls).not.toContain('run_speedtest')
      const plain = mount(React.createElement(SpeedTest))
      await plain.show(React.createElement(SpeedTest))
      expect(plain.container.textContent).not.toContain('999')
      expect(plain.container.textContent).toContain('not available in this build')
      expect(buttons(plain.container)).not.toContain('Run test')
      await React.act(async () => plain.root.unmount())
    } finally {
      if (previousDev === undefined) delete (import.meta.env as { DEV?: unknown }).DEV
      else (import.meta.env as { DEV?: unknown }).DEV = previousDev
      delete (globalThis as Record<string, unknown>).__bbV2Board
      scriptInvoke()
    }
  })
})
