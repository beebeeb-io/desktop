/**
 * The Finder status screen of the signed-out Beebeeb window (task 1683 slice 2; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.5, spec A `2026-10-06-macos-finder-setup-reconciler.md` §6.2,
 * §10). Ported from `tests/onboardingFinderStep.test.tsx`, where it tested `MacFinderStep` of the retired macOS
 * onboarding window: every assertion of that block is here, against `FinderScreen`, with the one change the spec makes:
 * on Ready the window moves on to Home by itself (`onReady`), where the step used to advance to the offline folders.
 *
 * `FinderScreen` consumes `useFinderSetup()` (lead ruling 7b), so these tests run the REAL hook declaration inside the
 * mounted component (the harness's `hookModules`), against a scripted backend and a scripted event bus. Lead ruling
 * 7a/c5: the `unavailable` presentation's "Try again" re-reads (`retry()`), a failure notice's "Try again" asks the
 * reconciler (`run('try_again')`); both are pinned against each other below.
 *
 * What this proves: the component's real handlers and render output for scripted answers. What it does NOT prove:
 * layout, React scheduling, a real Tauri event loop (tests/render-mac-app.mjs, the lead's device pass).
 */
import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, test } from 'bun:test'
import * as ReactReal from 'react'
import { createElement, Fragment } from 'react'
import { renderToString } from 'react-dom/server'
import * as desktopApi from '../src/desktopApi'
import * as finderSetup from '../src/finderSetup'
import * as copy from '../src/finderSetupCopy'
import * as accountCopy from '../src/accountViewCopy'
import * as parts from '../src/macSettingsParts'
import { FINDER_FAILURE_REASONS, type FinderSetupView } from '../src/finderSetup'
import { ToastProvider } from '../src/windows/ui'
import { loadComponent, mount, textOf, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const SignInCard = loadComponent('macSignIn.tsx', 'SignInCard', { React })
const btnClass = loadComponent('macSignIn.tsx', 'btnClass', {})
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

function openFinderScreen(initial: FinderSetupView | 'unreadable', backend: Record<string, (a: any) => unknown> = {}) {
  const bus = fakeBus()
  let done = 0
  const copied: string[] = []
  const m = mount('macSignIn.tsx', 'FinderScreen', {
    expand: true,
    backend: {
      finder_setup_state: () => { if (initial === 'unreadable') throw new Error('no reconciler'); return initial },
      finder_setup_copy_details: () => 'details',
      ...backend,
    },
    props: { onReady: () => { done += 1 } },
    hookModules: [{
      file: 'finderSetup.ts',
      name: 'useFinderSetup',
      bindings: {
        loadFinderSetup: finderSetup.loadFinderSetup,
        runFinderSetupAction: (action: copy.FinderSetupAction) =>
          finderSetup.runFinderSetupAction(action, { writeClipboard: async (text) => { copied.push(await text) } }),
        finderSetupLoadPresentation: copy.finderSetupLoadPresentation,
        FINDER_ACTION_FAILED: copy.FINDER_ACTION_FAILED,
        FINDER_COPIED_MS: copy.FINDER_COPIED_MS,
        FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
        commandUnavailableLabel: desktopApi.commandUnavailableLabel,
        subscribeFinderSetup: bus.subscribeFinderSetup,
        loadEngineRefusal: desktopApi.loadEngineRefusal,
      },
    }],
    bindings: {
      ...accountCopy,
      SignInCard,
      btnClass,
      Note: parts.Note,
      Btn: parts.Btn,
      useWindowTitle: () => {},
      finderActionButtonLabel: copy.finderActionButtonLabel,
    },
  })
  mounted.push(m)
  const settle = async () => { await m.flush(); await tick(); await m.flush() }
  const calls = (name: string) => m.calls.filter((c) => c.name === name).length
  return { m, bus, settle, calls, done: () => done, copied }
}

/** The reconciler's actions. “Continue to Home” (review P3-6, the lead's ruling of 2026-10-25) is the window's own way on
 *  while Finder is not Ready; it is not one of them, and `continueButtons` reads it. */
const allButtons = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
const buttons = (m: Mounted) => allButtons(m).filter((label) => label !== 'Continue to Home')
const continueButtons = (m: Mounted) => allButtons(m).filter((label) => label === 'Continue to Home')
const notices = (m: Mounted) => m.elements().filter((el) => el.props.role === 'alert' || el.props.role === 'status')
const pageText = (m: Mounted) => textOf(m.elements()[0])
const allStates: Array<FinderSetupView | 'unreadable'> = [
  view({ setup: 'missing' }),
  view(),
  view({ setup: 'ready' }),
  'unreadable',
  ...FINDER_FAILURE_REASONS.map(failedView),
]
describe('FinderScreen, with the real useFinderSetup hook', () => {
  test('shows Adding with no button, then moves the window on by itself on a Ready event', async () => {
    const s = openFinderScreen(view())
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

  // Review P3-6: with the real hook, every state that is not Ready has the way on; Ready moves on by itself and has none.
  test('“Continue to Home” is there in every state but Ready, and pressing it moves the window on once (P3-6)', async () => {
    for (const state of allStates) {
      const s = openFinderScreen(state)
      await s.settle()
      const isReady = state !== 'unreadable' && state.setup === 'ready'
      expect(continueButtons(s.m)).toEqual(isReady ? [] : ['Continue to Home'])
      if (isReady) continue
      const press = s.m.elements().find((el) => el.type === 'button' && textOf(el.props.children).trim() === 'Continue to Home')!
      await press.props.onClick()
      await s.settle()
      expect(s.done()).toBe(1)
      await press.props.onClick()
      expect(s.done()).toBe(1)
    }
  })

  // Must-render row 9: a failed + unknown with an engine refusal says the refusal's sentence.
  test('an engine refusal on a failed + unknown is the notice; engine_stop_unconfirmed has no button', async () => {
    const sentence = 'Beebeeb’s sync didn’t confirm it stopped. Quit and reopen Beebeeb before syncing again.'
    const s = openFinderScreen(failedView('unknown'), {
      sync_status: () => ({ logged_in: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: { code: 'engine_stop_unconfirmed', sentence } }),
    })
    await s.settle()
    expect(notices(s.m)).toHaveLength(1)
    expect(textOf(notices(s.m)[0].props.children)).toContain(sentence)
    expect(buttons(s.m)).toEqual([])
  })

  test('a Ready that is already there when the screen opens moves on too', async () => {
    const s = openFinderScreen(view({ setup: 'ready' }))
    await s.settle()
    expect(s.done()).toBe(1)
  })

  test('each reason renders exactly one notice and the action from §6.2', async () => {
    for (const reason of FINDER_FAILURE_REASONS) {
      const s = openFinderScreen(failedView(reason))
      await s.settle()
      expect(notices(s.m)).toHaveLength(1)
      // Spec §7.5 and the hi-fi (“If it fails”): a failure AND a turned-off extension are drawn ink on the error wash. The
      // shipped step drew “turned off” as a neutral status; the two-surface design replaces that, on this screen only.
      expect(notices(s.m)[0].props.role).toBe('alert')
      expect(textOf(notices(s.m)[0].props.children)).toContain(copy.FINDER_REASON_COPY[reason].sentence)
      expect(buttons(s.m)).toEqual([copy.FINDER_ACTION_LABEL[copy.FINDER_REASON_COPY[reason].action]])
      expect(s.m.toasts).toEqual([])
      expect(s.done()).toBe(0)
    }
  })

  test('no Install or Add to Finder button in any state, and none of the install-era commands is called', async () => {
    for (const state of allStates) {
      const s = openFinderScreen(state)
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
      const s = openFinderScreen(failedView(reason), { finder_setup_retry: () => undefined, open_login_items_and_extensions_settings: () => undefined, finder_setup_show_app: () => undefined })
      await s.settle()
      await s.m.click(label)
      expect(s.calls(command)).toBe(1)
      expect(s.m.toasts).toEqual([])
    }
  })

  test('Copy details says Copied on its button after a success (FT-clipboard)', async () => {
    const s = openFinderScreen(failedView('folder_taken'))
    await s.settle()
    await s.m.click('Copy details')
    await s.settle()
    expect(buttons(s.m)).toEqual(['Copied'])
  })

  test('Copy details puts the details on the pasteboard', async () => {
    const s = openFinderScreen(failedView('folder_taken'))
    await s.settle()
    await s.m.click('Copy details')
    expect(s.copied).toEqual(['details'])
  })

  test('a failed action is a toast, never a second inline surface', async () => {
    const s = openFinderScreen(failedView('timeout'), { finder_setup_retry: () => { throw new Error('finder_setup_retry is not a registered command') } })
    await s.settle()
    await s.m.click('Try again')
    expect(s.m.toasts.map((t) => t.message)).toEqual(['Beebeeb couldn’t retry adding itself to Finder.'])
    expect(s.m.toasts.map((t) => t.title)).toEqual([undefined])
    expect(JSON.stringify(s.m.toasts)).not.toContain('registered')
    expect(s.m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    expect(visibleErrorSurfaces(s.m).filter((surface) => surface.startsWith('inline:'))).toHaveLength(1)
  })

  // Row 15: Rust's own answer to Try again carries the remedy, so it is said, neutrally, under the notice.
  test('a failed Try again says Rust\'s sentence as a neutral note under the notice, no toast', async () => {
    const sentence = 'Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.'
    const s = openFinderScreen(failedView('timeout'), { finder_setup_retry: () => { throw sentence } })
    await s.settle()
    await s.m.click('Try again')
    expect(s.m.toasts).toEqual([])
    expect(s.m.elements().filter((el) => el.props.role === 'status').map((el) => textOf(el.props.children))).toContain(sentence)
    expect(visibleErrorSurfaces(s.m).filter((surface) => surface.startsWith('inline:'))).toHaveLength(1)
  })

  test('the action is disabled while it runs, so it cannot be sent twice', async () => {
    const s = openFinderScreen(failedView('timeout'), { finder_setup_retry: () => new Promise(() => {}) })
    await s.settle()
    await s.m.clickNoWait('Try again')
    const button = s.m.elements().find((el) => el.type === 'button')!
    expect(button.props.disabled).toBe(true)
  })

  describe('lead ruling 7a: a state that cannot be read is not "Adding"', () => {
    test('one inline notice with the existing words and ONE action, never the Adding line', async () => {
      const s = openFinderScreen('unreadable')
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
      const s = openFinderScreen('unreadable', {
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
      const s = openFinderScreen(failedView('timeout'), { finder_setup_retry: () => undefined })
      await s.settle()
      expect(s.calls('finder_setup_state')).toBe(1)
      await s.m.click('Try again')
      await s.settle()
      expect(s.calls('finder_setup_retry')).toBe(1)
      expect(s.calls('finder_setup_state')).toBe(1)
    })
  })

  test('before the reconciler has said anything, and while it says Missing, nothing claims Adding and nothing is offered', async () => {
    const waiting = openFinderScreen(view(), { finder_setup_state: () => new Promise(() => {}) })
    await waiting.settle()
    const missing = openFinderScreen(view({ setup: 'missing' }))
    await missing.settle()
    for (const s of [waiting, missing]) {
      expect(pageText(s.m)).not.toContain(copy.FINDER_ADDING_LINE)
      expect(buttons(s.m)).toEqual([])
      expect(notices(s.m)).toEqual([])
      expect(pageText(s.m)).toContain(copy.FINDER_SETUP_TITLE)
    }
  })

  test('one subscription while the step is open, released when it goes', async () => {
    const s = openFinderScreen(view())
    await s.settle()
    s.bus.emit(failedView('timeout'))
    await s.settle()
    expect(s.bus.live).toBe(1)
    s.m.unmount()
    expect(s.bus.live).toBe(0)
  })
})

describe('FinderScreen in real React', () => {
  test('the real hook, the real ToastProvider and the screen compose: the first paint is quiet, not "Adding"', () => {
    // Server render: it runs every hook in React's own order (what the controlled-hook harness
    // cannot show) but no effects, so this is the instant before the reconciler has answered.
    const FinderScreen = loadComponent('macSignIn.tsx', 'FinderScreen', {
      React,
      useState: ReactReal.useState,
      useEffect: ReactReal.useEffect,
      useRef: ReactReal.useRef,
      ...accountCopy,
      SignInCard,
      btnClass,
      Note: parts.Note,
      Btn: parts.Btn,
      useWindowTitle: () => {},
      finderActionButtonLabel: copy.finderActionButtonLabel,
      useFinderSetup: finderSetup.useFinderSetup,
    })
    const html = renderToString(createElement(ToastProvider, null, createElement(FinderScreen, { onReady: () => {} })))
    expect(html).toContain(copy.FINDER_SETUP_TITLE)
    expect(html).not.toContain('Adding')
    // The reconciler's own actions are absent; the window's way on (review P3-6) is the one button there is.
    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).toContain('>Continue to Home</button>')
  })
})

describe('FinderScreen source contract', () => {
  const source = readFileSync(new URL('../src/macSignIn.tsx', import.meta.url), 'utf8')
  const start = source.indexOf('export function FinderScreen(')
  const body = source.slice(start, source.indexOf('\n}\n', start))

  test('it takes the load, the subscription and the failed-action toast from useFinderSetup (ruling 7b)', () => {
    expect(body.includes('useFinderSetup(')).toBe(true)
    const own = ['loadFinderSetup', 'subscribeFinderSetup', 'runFinderSetupAction', 'useToast', 'showToast', 'listen(']
    expect(own.filter((name) => body.includes(name))).toEqual([])
  })

  test('no install-era command is named anywhere in the window’s sign-in screens', () => {
    expect(FORBIDDEN.filter((name) => source.includes(name))).toEqual([])
  })

  // Spec §7.5 said “‘Continue’ is not offered: Home already shows the Finder row's state”. The lead's ruling of 2026-10-25
  // (review P3-6) amends it: while Finder is not Ready the card offers ONE secondary “Continue to Home”, and no other way
  // on (the install and “continue without” commands of the old Finder step stay absent, above).
  test('the only “Continue” on the Finder screen is “Continue to Home”, from the copy module (P3-6)', () => {
    expect(body).toContain('FINDER_CONTINUE_TO_HOME')
    expect(body.replace(/FINDER_CONTINUE_TO_HOME/g, '')).not.toMatch(/Continue/i)
  })
})
