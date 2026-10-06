/**
 * Spec 2026-10-06 §10, Onboarding: on macOS there is no install button; the step shows Adding,
 * advances by itself on Ready, and on Failed / UserDisabled shows the one notice and one action
 * from finderSetupCopy.ts. On Linux/Windows the old step is unchanged.
 *
 * The macOS step consumes `useFinderSetup()` (lead ruling 7b), so these tests run the REAL hook
 * declaration inside the mounted component (the harness's `hookModules`), against a scripted
 * backend and a scripted event bus. Lead ruling 7a/c5: the `unavailable` presentation's "Try again"
 * re-reads (`retry()`), a failure notice's "Try again" asks the reconciler (`run('try_again')`);
 * both are pinned against each other below.
 *
 * What this proves: the component's real handlers and render output for scripted answers. What it
 * does NOT prove: layout, React scheduling, a real Tauri event loop (Playwright / Task 21).
 */
import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, test } from 'bun:test'
import * as ReactReal from 'react'
import { createElement, Fragment } from 'react'
import { renderToString } from 'react-dom/server'
import ts from 'typescript'
import * as desktopApi from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import * as finderSetup from '../src/finderSetup'
import * as copy from '../src/finderSetupCopy'
import { FINDER_FAILURE_REASONS, type FinderSetupView } from '../src/finderSetup'
import { ToastProvider } from '../src/windows/ui'
import { elementsOf, loadComponent, mount, textOf, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const Wordmark = () => null
const Card = loadComponent('Onboarding.tsx', 'Card', { React, Wordmark })
const FORBIDDEN = ['install_finder_location', 'continue_without_finder_location', 'finder_location_state', 'finder_domain_user_enabled']

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'adding',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 0,
  last_failure: null,
  ...over,
})
const failedView = (reason: (typeof FINDER_FAILURE_REASONS)[number]) =>
  view({ setup: reason === 'user_disabled' ? 'user_disabled' : 'failed', reason })

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

/** A scripted stand-in for `subscribeFinderSetup`; the registration lands on the next microtask. */
function fakeBus() {
  const listeners: Array<(v: FinderSetupView) => void> = []
  return {
    get live() { return listeners.length },
    subscribeFinderSetup(onView: (v: FinderSetupView) => void, options: { onSubscribed?: () => void } = {}) {
      listeners.push(onView)
      void Promise.resolve().then(() => options.onSubscribed?.())
      return () => { const at = listeners.indexOf(onView); if (at >= 0) listeners.splice(at, 1) }
    },
    emit(next: FinderSetupView) { for (const listener of [...listeners]) listener(next) },
  }
}

function openMacStep(initial: FinderSetupView | 'unreadable', backend: Record<string, (a: any) => unknown> = {}) {
  const bus = fakeBus()
  let done = 0
  const copied: string[] = []
  const m = mount('Onboarding.tsx', 'MacFinderStep', {
    expand: true,
    backend: {
      finder_setup_state: () => { if (initial === 'unreadable') throw new Error('no reconciler'); return initial },
      finder_setup_copy_details: () => 'details',
      ...backend,
    },
    props: { onDone: () => { done += 1 } },
    hookModules: [{
      file: 'finderSetup.ts',
      name: 'useFinderSetup',
      bindings: {
        loadFinderSetup: finderSetup.loadFinderSetup,
        runFinderSetupAction: (action: copy.FinderSetupAction) =>
          finderSetup.runFinderSetupAction(action, { writeClipboard: async (text) => { copied.push(text) } }),
        finderSetupLoadPresentation: copy.finderSetupLoadPresentation,
        FINDER_ACTION_FAILED: copy.FINDER_ACTION_FAILED,
        FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
        commandUnavailableLabel: desktopApi.commandUnavailableLabel,
        subscribeFinderSetup: bus.subscribeFinderSetup,
      },
    }],
    bindings: { Card, FINDER_SETUP_TITLE: copy.FINDER_SETUP_TITLE },
  })
  mounted.push(m)
  const settle = async () => { await m.flush(); await tick(); await m.flush() }
  const calls = (name: string) => m.calls.filter((c) => c.name === name).length
  return { m, bus, settle, calls, done: () => done, copied }
}

const buttons = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
const notices = (m: Mounted) => m.elements().filter((el) => el.props.role === 'alert' || el.props.role === 'status')
const pageText = (m: Mounted) => textOf(m.elements()[0])
const allStates: Array<FinderSetupView | 'unreadable'> = [
  view({ setup: 'missing' }),
  view(),
  view({ setup: 'ready' }),
  'unreadable',
  ...FINDER_FAILURE_REASONS.map(failedView),
]

describe('Onboarding Finder step on macOS', () => {
  test('shows Adding with no button, then advances by itself on a Ready event', async () => {
    const s = openMacStep(view())
    await s.settle()
    expect(pageText(s.m)).toContain('Adding Beebeeb to Finder…')
    expect(buttons(s.m)).toEqual([])
    expect(s.done()).toBe(0)
    s.bus.emit(view({ setup: 'ready' }))
    await s.settle()
    expect(s.done()).toBe(1)
    await s.settle()
    expect(s.done()).toBe(1)
  })

  test('a Ready that is already there when the step opens advances too', async () => {
    const s = openMacStep(view({ setup: 'ready' }))
    await s.settle()
    expect(s.done()).toBe(1)
  })

  test('each reason renders exactly one notice and the action from §6.2', async () => {
    for (const reason of FINDER_FAILURE_REASONS) {
      const s = openMacStep(failedView(reason))
      await s.settle()
      expect(notices(s.m)).toHaveLength(1)
      expect(notices(s.m)[0].props.role).toBe(reason === 'user_disabled' ? 'status' : 'alert')
      expect(textOf(notices(s.m)[0].props.children)).toContain(copy.FINDER_REASON_COPY[reason].sentence)
      expect(buttons(s.m)).toEqual([copy.FINDER_ACTION_LABEL[copy.FINDER_REASON_COPY[reason].action]])
      expect(s.m.toasts).toEqual([])
      expect(s.done()).toBe(0)
    }
  })

  test('no Install or Add to Finder button in any state, and none of the install-era commands is called', async () => {
    for (const state of allStates) {
      const s = openMacStep(state)
      await s.settle()
      expect(buttons(s.m).join('|')).not.toMatch(/Install|Add to Finder|Continue without/)
      expect(s.m.calls.map((c) => c.name).filter((name) => FORBIDDEN.includes(name))).toEqual([])
    }
  })

  test('each action sends its own command: Try again, Open System Settings, Show in Finder, Copy details', async () => {
    const cases: Array<[typeof FINDER_FAILURE_REASONS[number], string, string]> = [
      ['timeout', 'Try again', 'finder_setup_retry'],
      ['user_disabled', 'Open System Settings', 'open_login_items_and_extensions_settings'],
      ['not_in_applications', 'Show in Finder', 'finder_setup_show_app'],
      ['folder_taken', 'Copy details', 'finder_setup_copy_details'],
    ]
    for (const [reason, label, command] of cases) {
      const s = openMacStep(failedView(reason), { finder_setup_retry: () => undefined, open_login_items_and_extensions_settings: () => undefined, finder_setup_show_app: () => undefined })
      await s.settle()
      await s.m.click(label)
      expect(s.calls(command)).toBe(1)
      expect(s.m.toasts).toEqual([])
    }
  })

  test('Copy details puts the details on the pasteboard', async () => {
    const s = openMacStep(failedView('folder_taken'))
    await s.settle()
    await s.m.click('Copy details')
    expect(s.copied).toEqual(['details'])
  })

  test('a failed action is a toast, never a second inline surface', async () => {
    const s = openMacStep(failedView('timeout'), { finder_setup_retry: () => { throw new Error('bridge down') } })
    await s.settle()
    await s.m.click('Try again')
    expect(s.m.toasts.map((t) => t.message)).toEqual(['Beebeeb couldn’t retry adding itself to Finder.'])
    expect(s.m.toasts.map((t) => t.title)).toEqual([undefined])
    expect(JSON.stringify(s.m.toasts)).not.toContain('bridge down')
    expect(s.m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    expect(visibleErrorSurfaces(s.m).filter((surface) => surface.startsWith('inline:'))).toHaveLength(1)
  })

  test('the action is disabled while it runs, so it cannot be sent twice', async () => {
    const s = openMacStep(failedView('timeout'), { finder_setup_retry: () => new Promise(() => {}) })
    await s.settle()
    await s.m.clickNoWait('Try again')
    const button = s.m.elements().find((el) => el.type === 'button')!
    expect(button.props.disabled).toBe(true)
  })

  describe('lead ruling 7a: a state that cannot be read is not "Adding"', () => {
    test('one inline notice with the existing words and ONE action, never the Adding line', async () => {
      const s = openMacStep('unreadable')
      await s.settle()
      expect(notices(s.m)).toHaveLength(1)
      expect(textOf(notices(s.m)[0].props.children)).toContain(copy.FINDER_UNAVAILABLE_LINE)
      expect(buttons(s.m)).toEqual(['Try again'])
      expect(pageText(s.m)).not.toContain(copy.FINDER_ADDING_LINE)
      expect(s.m.toasts).toEqual([])
      expect(s.done()).toBe(0)
    })

    test('its Try again re-reads the state (retry) and never asks the reconciler (run try_again)', async () => {
      let reads = 0
      const s = openMacStep('unreadable', {
        finder_setup_state: () => { reads += 1; if (reads === 1) throw new Error('not yet'); return view() },
        finder_setup_retry: () => undefined,
      })
      await s.settle()
      expect(s.calls('finder_setup_state')).toBe(1)
      await s.m.click('Try again')
      await s.settle()
      expect(s.calls('finder_setup_state')).toBe(2)
      expect(s.calls('finder_setup_retry')).toBe(0)
      expect(pageText(s.m)).toContain(copy.FINDER_ADDING_LINE)
      expect(buttons(s.m)).toEqual([])
    })

    test('a failure notice Try again asks the reconciler (run try_again) and never re-reads the state', async () => {
      const s = openMacStep(failedView('timeout'), { finder_setup_retry: () => undefined })
      await s.settle()
      expect(s.calls('finder_setup_state')).toBe(1)
      await s.m.click('Try again')
      await s.settle()
      expect(s.calls('finder_setup_retry')).toBe(1)
      expect(s.calls('finder_setup_state')).toBe(1)
    })
  })

  test('before the reconciler has said anything, and while it says Missing, nothing claims Adding and nothing is offered', async () => {
    const waiting = openMacStep(view(), { finder_setup_state: () => new Promise(() => {}) })
    await waiting.settle()
    const missing = openMacStep(view({ setup: 'missing' }))
    await missing.settle()
    for (const s of [waiting, missing]) {
      expect(pageText(s.m)).not.toContain(copy.FINDER_ADDING_LINE)
      expect(buttons(s.m)).toEqual([])
      expect(notices(s.m)).toEqual([])
      expect(pageText(s.m)).toContain(copy.FINDER_SETUP_TITLE)
    }
  })

  test('one subscription while the step is open, released when it closes', async () => {
    const s = openMacStep(view())
    await s.settle()
    s.bus.emit(failedView('timeout'))
    await s.settle()
    expect(s.bus.live).toBe(1)
    s.m.unmount()
    expect(s.bus.live).toBe(0)
  })
})

describe('Onboarding Finder step in real React', () => {
  test('the real hook, the real ToastProvider and the step compose: the first paint is quiet, not "Adding"', () => {
    // Server render: it runs every hook in React's own order (what the controlled-hook harness
    // cannot show) but no effects, so this is the instant before the reconciler has answered.
    const MacFinderStep = loadComponent('Onboarding.tsx', 'MacFinderStep', {
      React,
      useState: ReactReal.useState,
      useEffect: ReactReal.useEffect,
      Card,
      useFinderSetup: finderSetup.useFinderSetup,
      FINDER_SETUP_TITLE: copy.FINDER_SETUP_TITLE,
    })
    const html = renderToString(createElement(ToastProvider, null, createElement(MacFinderStep, { onDone: () => {} })))
    expect(html).toContain(copy.FINDER_SETUP_TITLE)
    expect(html).not.toContain('Adding')
    expect(html).not.toContain('<button')
  })
})

describe('Onboarding Finder step source contract', () => {
  const source = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
  const ast = ts.createSourceFile('Onboarding.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const textOfFunction = (name: string) => {
    const declaration = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
    if (!declaration) throw new Error(`Missing function ${name} in Onboarding.tsx`)
    return declaration.getText(ast)
  }

  test('MacFinderStep takes the load, the subscription and the failed-action toast from useFinderSetup (ruling 7b)', () => {
    const body = textOfFunction('MacFinderStep')
    expect(body.includes('useFinderSetup(')).toBe(true)
    const own = ['loadFinderSetup', 'subscribeFinderSetup', 'runFinderSetupAction', 'useToast', 'showToast', 'listen(']
    expect(own.filter((name) => body.includes(name))).toEqual([])
  })

  test('the install-era commands and the poll live only in the Windows/Linux step', () => {
    const rest = source.replace(textOfFunction('FinderInstallStep'), '')
    expect(FORBIDDEN.filter((name) => rest.includes(name))).toEqual([])
    const gone = ['USER_ENABLED_POLL_INTERVAL_MS', 'shouldRetryAfterUserEnabledPoll', 'finder_domain_user_enabled']
    expect(gone.filter((name) => source.includes(name))).toEqual([])
    expect(textOfFunction('FinderInstallStep').includes('desktop_platform')).toBe(false)
  })
})

function stubs() {
  const make = (name: string) => Object.defineProperty(function () { return null }, 'name', { value: name }) as (props: any) => null
  return {
    SignInStep: make('SignInStep'),
    UnlockStep: make('UnlockStep'),
    MacFinderStep: make('MacFinderStep'),
    FinderInstallStep: make('FinderInstallStep'),
    PinningStep: make('PinningStep'),
    ReadyStep: make('ReadyStep'),
  }
}

async function openOnboarding(
  backend: Record<string, (a: any) => unknown>,
  opts: { rail?: unknown[]; hostOs?: desktopApi.DesktopPlatform } = {},
) {
  const steps = stubs()
  const m = mount('Onboarding.tsx', 'OnboardingView', {
    backend,
    bindings: {
      ...desktopApi,
      ...finderSetup,
      ...steps,
      Wordmark,
      STEPS: opts.rail ?? [],
      // The capability snapshot (main.tsx's CapabilityProvider); null = no snapshot.
      useCapabilities: () => (opts.hostOs === undefined ? null : { host_os: opts.hostOs }),
      FINDER_RAIL_TITLE: copy.FINDER_RAIL_TITLE,
      FINDER_RAIL_DETAIL: copy.FINDER_RAIL_DETAIL,
      useRegionLabel: () => 'the EU',
    },
  })
  mounted.push(m)
  await m.flush(); await tick(); await m.flush()
  const shown = () => Object.entries(steps).filter(([, type]) => m.elements().some((el) => el.type === type)).map(([name]) => name)
  const props = (name: keyof typeof steps) => m.elements().find((el) => el.type === steps[name])!.props
  return { m, shown, props }
}

const signedIn = (over: Partial<desktopApi.SyncStatus> = {}) => ({ logged_in: true, engine: 'idle', sync_root: null, vault_unlocked: true, syncing: 0, cloud_only: 0, conflicts: 0, ...over })

describe('Onboarding routes to the Finder step by the reconciler on macOS', () => {
  test('macOS, keys present, Finder Ready: straight to the last step, without asking the install-era state', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => 'macos', finder_setup_state: () => view({ setup: 'ready' }) })
    expect(o.shown()).toEqual(['ReadyStep'])
    expect(o.m.calls.map((c) => c.name).filter((name) => FORBIDDEN.includes(name))).toEqual([])
  })

  test('macOS, keys present, Finder not Ready (adding, failed, or unreadable): the Finder step, the macOS one', async () => {
    for (const state of [view(), failedView('timeout'), failedView('user_disabled'), 'unreadable' as const]) {
      const o = await openOnboarding({
        sync_status: () => signedIn(),
        desktop_platform: () => 'macos',
        finder_setup_state: () => { if (state === 'unreadable') throw new Error('no reconciler'); return state },
      })
      expect(o.shown()).toEqual(['MacFinderStep'])
      expect(o.m.calls.map((c) => c.name).filter((name) => FORBIDDEN.includes(name))).toEqual([])
    }
  })

  test('macOS, vault still locked: the unlock step comes first', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn({ vault_unlocked: false }), desktop_platform: () => 'macos', finder_setup_state: () => view({ setup: 'ready' }) })
    expect(o.shown()).toEqual(['UnlockStep'])
  })

  test('signing in and unlocking lead to the macOS Finder step, which leads on to the offline folders', async () => {
    const o = await openOnboarding({ sync_status: () => ({ logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }), desktop_platform: () => 'macos' })
    expect(o.shown()).toEqual(['SignInStep'])
    o.props('SignInStep').onDone({ kind: 'fresh' }); o.m.render()
    expect(o.shown()).toEqual(['UnlockStep'])
    o.props('UnlockStep').onDone(); o.m.render()
    expect(o.shown()).toEqual(['MacFinderStep'])
    o.props('MacFinderStep').onDone(); o.m.render()
    expect(o.shown()).toEqual(['PinningStep'])
  })

  test('Windows and Linux keep the old rule (a sync folder decides) and the old step, without the reconciler', async () => {
    for (const platform of ['windows', 'linux']) {
      const without = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => platform })
      expect(without.shown()).toEqual(['FinderInstallStep'])
      const withRoot = await openOnboarding({ sync_status: () => signedIn({ sync_root: '/home/sam/Beebeeb' }), desktop_platform: () => platform })
      expect(withRoot.shown()).toEqual(['ReadyStep'])
      for (const o of [without, withRoot]) expect(o.m.calls.map((c) => c.name)).not.toContain('finder_setup_state')
    }
  })

  test('a platform that cannot be read, with no capability snapshot to ask, is the Windows/Linux step, not a blank page', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => { throw new Error('no bridge') } })
    expect(o.shown()).toEqual(['FinderInstallStep'])
  })

  test('reaching the Finder step before the platform is known shows neither step, then the right one (no flash of the wrong one)', async () => {
    // The step can be reached by walking sign in -> unlock while `desktop_platform` is still
    // pending (the mount read waits for both), so this is the only path the guard protects.
    let resolve!: (platform: string) => void
    const o = await openOnboarding({
      sync_status: () => ({ logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }),
      desktop_platform: () => new Promise((r) => { resolve = r as (platform: string) => void }),
    })
    o.props('SignInStep').onDone({ kind: 'fresh' }); o.m.render()
    o.props('UnlockStep').onDone(); o.m.render()
    expect(o.shown()).toEqual([])
    resolve('macos')
    await o.m.flush(); await tick(); await o.m.flush()
    expect(o.shown()).toEqual(['MacFinderStep'])
  })
})

describe('Onboarding platform fallback: a Mac whose desktop_platform fails is still a Mac (Global Constraint: on macOS no code path calls install_finder_location)', () => {
  const failing = () => { throw new Error('no bridge') }
  const forbiddenCalls = (o: { m: Mounted }) => o.m.calls.map((c) => c.name).filter((name) => FORBIDDEN.includes(name))

  test('desktop_platform fails, the capability snapshot says macos: the macOS step and no install-era command', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: failing, finder_setup_state: () => view() }, { hostOs: 'macos' })
    expect(o.shown()).toEqual(['MacFinderStep'])
    expect(forbiddenCalls(o)).toEqual([])
  })

  test('the routing read follows the fallback too: Ready goes straight to the last step, without the install-era state', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: failing, finder_setup_state: () => view({ setup: 'ready' }) }, { hostOs: 'macos' })
    expect(o.shown()).toEqual(['ReadyStep'])
    expect(forbiddenCalls(o)).toEqual([])
  })

  test('desktop_platform answering "unknown" falls back the same way', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => 'unknown', finder_setup_state: () => view() }, { hostOs: 'macos' })
    expect(o.shown()).toEqual(['MacFinderStep'])
    expect(forbiddenCalls(o)).toEqual([])
  })

  test('desktop_platform fails, the snapshot says windows or linux: today\'s step', async () => {
    for (const hostOs of ['windows', 'linux'] as const) {
      const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: failing }, { hostOs })
      expect(o.shown()).toEqual(['FinderInstallStep'])
      expect(o.m.calls.map((c) => c.name)).not.toContain('finder_setup_state')
    }
  })

  test('both unknown (failed or "unknown" answer; an "unknown" snapshot or none): only then the Windows/Linux step', async () => {
    const answers: Array<() => unknown> = [failing, () => 'unknown']
    for (const answer of answers) {
      for (const hostOs of ['unknown', undefined] as const) {
        const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: answer }, { hostOs })
        expect(o.shown()).toEqual(['FinderInstallStep'])
      }
    }
  })

  test('the fallback is only a fallback: an answered platform wins over the snapshot', async () => {
    for (const answered of ['windows', 'linux']) {
      const o = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => answered }, { hostOs: 'macos' })
      expect(o.shown()).toEqual(['FinderInstallStep'])
    }
    const mac = await openOnboarding({ sync_status: () => signedIn(), desktop_platform: () => 'macos', finder_setup_state: () => view() }, { hostOs: 'windows' })
    expect(mac.shown()).toEqual(['MacFinderStep'])
  })

  test('signing in and unlocking on a Mac whose desktop_platform fails also lands on the macOS step', async () => {
    const o = await openOnboarding(
      { sync_status: () => ({ logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }), desktop_platform: failing },
      { hostOs: 'macos' },
    )
    o.props('SignInStep').onDone({ kind: 'fresh' }); o.m.render()
    o.props('UnlockStep').onDone(); o.m.render()
    expect(o.shown()).toEqual(['MacFinderStep'])
    expect(forbiddenCalls(o)).toEqual([])
  })
})

describe('Onboarding step rail: macOS promises no manual install (lead ruling, pre-review of Task 15)', () => {
  /** The module-level `STEPS` constant of Onboarding.tsx, evaluated as written (it is not exported). */
  function realSteps(): unknown[] {
    const source = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
    const ast = ts.createSourceFile('Onboarding.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
    const statement = ast.statements.find((n) => ts.isVariableStatement(n) && n.declarationList.declarations.some((d) => d.name.getText(ast) === 'STEPS'))
    if (!statement) throw new Error('Missing const STEPS in Onboarding.tsx')
    const compiled = ts.transpileModule(statement.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
    return new Function(`${compiled}; return STEPS`)()
  }

  /** The rail as drawn: one [title, detail] pair per row, in order. */
  const rail = (m: Mounted) =>
    m.elements()
      .filter((el) => String(el.props.className ?? '').startsWith('step-row'))
      .map((row) => {
        const inner = elementsOf(row.props.children)
        const text = (cls: string) => textOf(inner.find((el) => el.props.className === cls)?.props.children).trim()
        return [text('row-title'), text('row-detail')]
      })

  // Written out here, not read from the source, so a change to either side shows up.
  const unchanged = [
    ['Sign in', 'Authenticate your account.'],
    ['Set up this Mac', 'Restore the vault key on this device.'],
    ['Install Finder location', 'Register Beebeeb in the Finder sidebar.'],
    ['Choose offline folders', 'Default is online-only; pin only what you need.'],
    ['Review status', 'Open the control center.'],
  ]
  const onMac = unchanged.map((row, at) => (at === 2 ? ['Finder', 'Beebeeb adds itself to Finder after sign-in.'] : row))
  const loggedOut = { logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }

  test('the two macOS strings are named exports of the copy module, and promise no install', () => {
    expect(copy.FINDER_RAIL_TITLE).toBe('Finder')
    expect(copy.FINDER_RAIL_DETAIL).toBe('Beebeeb adds itself to Finder after sign-in.')
    expect(`${copy.FINDER_RAIL_TITLE} ${copy.FINDER_RAIL_DETAIL}`).not.toMatch(/Install|Register|Add to Finder/)
  })

  test('the rail the source declares is the one the Windows/Linux strings below are held to', () => {
    expect(realSteps().map((row: any) => [row.title, row.detail])).toEqual(unchanged)
  })

  test('on macOS the Finder row says Finder and that it adds itself; every other row is unchanged', async () => {
    const o = await openOnboarding({ sync_status: () => loggedOut, desktop_platform: () => 'macos' }, { rail: realSteps() })
    expect(rail(o.m)).toEqual(onMac)
  })

  test('on Windows, Linux, an unknown platform and an unreadable one the rail is exactly today\'s', async () => {
    for (const platform of ['windows', 'linux', 'unknown']) {
      const o = await openOnboarding({ sync_status: () => loggedOut, desktop_platform: () => platform }, { rail: realSteps() })
      expect(rail(o.m)).toEqual(unchanged)
    }
    const unreadable = await openOnboarding({ sync_status: () => loggedOut, desktop_platform: () => { throw new Error('no bridge') } }, { rail: realSteps() })
    expect(rail(unreadable.m)).toEqual(unchanged)
  })

  test('the rail follows the fallback: desktop_platform fails and the snapshot says macos', async () => {
    const o = await openOnboarding(
      { sync_status: () => loggedOut, desktop_platform: () => { throw new Error('no bridge') } },
      { rail: realSteps(), hostOs: 'macos' },
    )
    expect(rail(o.m)).toEqual(onMac)
  })

  test('until the platform has answered the rail stays today\'s, then becomes the macOS one', async () => {
    let resolve!: (platform: string) => void
    const o = await openOnboarding(
      { sync_status: () => loggedOut, desktop_platform: () => new Promise((r) => { resolve = r as (platform: string) => void }) },
      { rail: realSteps() },
    )
    expect(rail(o.m)).toEqual(unchanged)
    resolve('macos')
    await o.m.flush(); await tick(); await o.m.flush()
    expect(rail(o.m)).toEqual(onMac)
  })
})

describe('Onboarding Finder step on Windows/Linux is unchanged', () => {
  function openWindowsStep(backend: Record<string, (a: any) => unknown>) {
    let done = 0
    const m = mount('Onboarding.tsx', 'FinderInstallStep', {
      backend: { default_sync_root: () => '/home/sam/Beebeeb', finder_location_state: () => ({ installed: false, status: 'missing' }), ...backend },
      props: { onDone: () => { done += 1 } },
      bindings: { ...desktopApi, ...finderInstallCard, Card },
    })
    mounted.push(m)
    return { m, done: () => done }
  }

  test('Install Finder location sends the chosen path; a failure offers Continue without install', async () => {
    const { m } = openWindowsStep({ install_finder_location: () => { throw new Error('File Provider is only available on macOS.') } })
    await m.flush()
    await m.click('Install Finder location')
    expect(m.calls.find((c) => c.name === 'install_finder_location')?.args).toEqual({ path: '/home/sam/Beebeeb' })
    expect(buttons(m)).toContain('Continue without install')
  })

  test('Continue without install carries the path on and moves the step along', async () => {
    const { m, done } = openWindowsStep({
      install_finder_location: () => { throw new Error('nope') },
      continue_without_finder_location: () => undefined,
    })
    await m.flush()
    await m.click('Install Finder location')
    await m.click('Continue without install')
    expect(m.calls.find((c) => c.name === 'continue_without_finder_location')?.args).toEqual({ path: '/home/sam/Beebeeb' })
    expect(done()).toBe(1)
  })

  test('before a failure there is no escape hatch, and the picker and the install button are the whole row', async () => {
    const { m } = openWindowsStep({})
    await m.flush()
    expect(buttons(m)).toEqual(['Choose location', 'Install Finder location'])
  })

  test('a successful install moves on; the macOS-only calls are gone from this step', async () => {
    const { m, done } = openWindowsStep({ install_finder_location: () => ({ installed: true, path: '/home/sam/Beebeeb', status: 'installed' }) })
    await m.flush()
    await m.click('Install Finder location')
    expect(done()).toBe(1)
    expect(m.calls.map((c) => c.name)).not.toContain('desktop_platform')
    expect(m.calls.map((c) => c.name)).not.toContain('finder_domain_user_enabled')
  })
})
