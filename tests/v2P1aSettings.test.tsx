// P1a: R1f's (#270) smaller follow-ups in Settings and the gate, run with the component harness against a scripted
// backend: Settings' first read never writes over a change made before it answers and shows no board numbers in the
// window, each sign-out sends one clear_session on a double click, and "Continue to Drive" opens the v2 window.
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { clearSession, command } from '../src/desktopApi'
import { elementsOf, mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const onDisk = { upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, local_cache_limit_bytes: 10 }
const DEFAULTS = { ...onDisk, local_cache_limit_bytes: 0 }
const BOARD = { ...onDisk, local_cache_limit_bytes: 50 }

describe('Settings’ first read (useConfig)', () => {
  test('a change made before the read answers is kept; the read fills the rest', async () => {
    let answer: (v: unknown) => void = () => {}
    const writes: unknown[] = []
    const m = mount('v2/sections/settings/shell/data.ts', 'useConfig', {
      backend: {
        get_desktop_config: () => new Promise((done) => { answer = done }),
        set_desktop_config: ({ config }: { config: unknown }) => { writes.push(config) },
      },
      bindings: { command, DEFAULT_CONFIG: DEFAULTS, FIXTURE_CONFIG: BOARD, hasTauri: () => true },
    })
    mounted.push(m)
    await m.flush()
    // Until the read answers, the window shows the app's own defaults, never the board's example numbers.
    expect(m.tree().config.local_cache_limit_bytes).toBe(0)
    expect(m.tree().loaded).toBe(false)
    m.tree().update({ notify_conflicts: false })
    await m.flush()
    answer({ ...onDisk, notify_conflicts: true })
    await m.flush()
    expect(m.tree().config.notify_conflicts).toBe(false)
    expect(m.tree().config.local_cache_limit_bytes).toBe(10)
    expect(m.tree().loaded).toBe(true)
    expect(writes).toEqual([{ notify_conflicts: false }])
  })
})

describe('one sign-out at a time', () => {
  test('the Settings confirm sends one clear_session for a double click', async () => {
    let calls = 0
    let done: (v: unknown) => void = () => {}
    const m = mount('v2/sections/settings/account/SecurityPane.tsx', 'SignOutDialog', {
      backend: { clear_session: () => { calls += 1; return new Promise((r) => { done = r }) } },
      bindings: { clearSession, hasTauri: () => true, Dialog: 'settings-dialog', Btn: 'settings-btn' },
      props: { all: false, onCancel() {} },
    })
    mounted.push(m)
    ;(globalThis as any).window.location = { assign() {} }
    const button = () => elementsOf(m.elements().find((el) => el.type === 'settings-dialog')!.props.actions).find((el) => el.props.variant === 'danger')!
    button().props.onClick()
    button().props.onClick()
    await m.flush()
    expect(calls).toBe(1)
    expect(button().props.disabled).toBe(true)
    done({ warning: null })
    await m.flush()
    expect(calls).toBe(1)
  })
  test('the no-plan screen sends one clear_session for a double click', async () => {
    let calls = 0
    let done: (v: unknown) => void = () => {}
    const m = mount('v2/auth/Gate.tsx', 'NoPlanWindow', {
      backend: { clear_session: () => { calls += 1; return new Promise((r) => { done = r }) } },
      bindings: { clearSession, command, PLANS_URL: 'https://beebeeb.io/pricing', NoPlanView: 'no-plan-view' },
      props: { onCheckAgain() {}, onSignedOut() {} },
    })
    mounted.push(m)
    const view = () => m.elements().find((el) => el.type === 'no-plan-view')!
    view().props.onSignOut()
    view().props.onSignOut()
    await m.flush()
    expect(calls).toBe(1)
    done({ warning: null })
    await m.flush()
  })
})

describe('"Continue to Drive" after a cancelled deletion opens the v2 window (#270 P3)', () => {
  test('the link carries platform=windows, as the sign-out does', () => {
    const pane = readFileSync(new URL('../src/v2/sections/settings/account/SecurityPane.tsx', import.meta.url), 'utf8')
    expect(pane).toContain("export const V2_WINDOW = '?window=main-app&platform=windows'")
    expect(pane).toContain('<a href={`${V2_WINDOW}&section=files`}')
    expect(pane).not.toContain('href="?window=main-app&section=files"')
  })
})
