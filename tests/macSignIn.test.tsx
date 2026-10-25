/**
 * The signed-out content of the Beebeeb window (task 1683 slice 2; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.4, §7.5, §8; spec C `…-first-run-sign-in.md` §7.1, §7.2; hi-fi `design/hifi/macos-two-surface.html` section 2).
 *
 * The real declarations of `src/macSignIn.tsx`, executed with the component harness against a scripted backend.
 *
 * What this proves: which screen each state shows, every word and control of each screen, the order and outcome of
 * each sign-in method, the account-switch step (task 1887: every pin of `reauthInPlace.test.tsx`’s AccountSwitchStep
 * block is carried over, with the same assertions, to the in-window step), the recovery phrase, the Finder status
 * screen, and the links and Quit. What it does NOT prove: layout (tests/render-mac-app.mjs), real focus, or anything
 * on a Mac (the lead’s device pass).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as accountSession from '../src/accountSession'
import * as desktopApi from '../src/desktopApi'
import * as copy from '../src/accountViewCopy'
import * as switchCopy from '../src/accountSwitchCopy'
import * as signOutCopy from '../src/signOutCopy'
import * as signIn from '../src/onboardingSignIn'
import * as model from '../src/signInModel'
import * as browserCopy from '../src/browserLoginCopy'
import * as finderCopy from '../src/finderSetupCopy'
import * as finderSetup from '../src/finderSetup'
import * as parts from '../src/macSettingsParts'
import { elementsOf, expand, loadComponent, mount, textOf, type Mounted, type TreeNode } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close(); windowTitles.length = 0; accountSession.clearSignOutWarning() })

/** Every title a card asked the window for, in order (the real `useWindowTitle` is tested at the end). */
const windowTitles: string[] = []
const titleSpy = (title: string) => { windowTitles.push(title) }
const btnClass = loadComponent('macSignIn.tsx', 'btnClass', {})
const displayAddress = loadComponent('macSignIn.tsx', 'displayAddress', {})
const SignInCard = loadComponent('macSignIn.tsx', 'SignInCard', { React })
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const button = (m: Mounted, label: string) => {
  const hit = m.elements().filter((el) => el.type === 'button' && textOf(el.props.children).trim() === label)
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}`)
  return hit[0]
}
const hasButton = (m: Mounted, label: string) => m.elements().some((el) => el.type === 'button' && textOf(el.props.children).trim() === label)
const buttonLabels = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
const classes = (el: TreeNode) => String(el.props.className ?? '').split(/\s+/).filter(Boolean)
const text = (m: Mounted) => textOf(expand(m.tree()))
const heading = (m: Mounted) => m.elements().find((el) => el.type === 'h1')

// ── The card ────────────────────────────────────────────────────────────────

describe('SignInCard: the window’s one card (hi-fi .scard)', () => {
  const card = (props: Record<string, unknown>) => expand(SignInCard({ children: createElement('p', null, 'body'), ...props }))
  const flat = (node: any) => elementsOf(node)

  test('a labelled section: the mark, the heading, the words, then the body', () => {
    const tree = card({ title: 'Sign in to Beebeeb', copy: 'Some words.' })
    expect(tree.type).toBe('section')
    expect(tree.props['aria-labelledby']).toBe('mw-title')
    const els = flat(tree)
    const mark = els.find((el) => classes(el).includes('mw-mark'))!
    expect(textOf(mark.props.children)).toBe('b')
    expect(mark.props['aria-hidden']).toBe('true')
    const h1 = els.find((el) => el.type === 'h1')!
    expect(h1.props.id).toBe('mw-title')
    expect(textOf(h1.props.children)).toBe('Sign in to Beebeeb')
    expect(textOf(tree)).toContain('Some words.')
    expect(textOf(tree).endsWith('body')).toBe(true)
  })

  test('no words, no paragraph', () => {
    expect(flat(card({ title: 'T' })).filter((el) => classes(el).includes('mw-scard-copy'))).toHaveLength(0)
  })

  test('the width is the hi-fi’s: 380 pt, 440 pt for the phrase and Finder, 460 pt for the switch', () => {
    expect(classes(card({ title: 'T' }))).toContain('mw-scard--base')
    expect(classes(card({ title: 'T', size: 'wide' }))).toContain('mw-scard--wide')
    expect(classes(card({ title: 'T', size: 'switch' }))).toContain('mw-scard--switch')
  })
})

describe('displayAddress: the address as a person reads it', () => {
  test('without the scheme, and nothing else changed (the pasteboard gets the whole address)', () => {
    expect(displayAddress('https://app.beebeeb.io/cli-auth')).toBe('app.beebeeb.io/cli-auth')
    expect(displayAddress('http://localhost:5173/cli-auth')).toBe('localhost:5173/cli-auth')
    expect(displayAddress('app.beebeeb.io/cli-auth')).toBe('app.beebeeb.io/cli-auth')
    expect(displayAddress('https://app.beebeeb.io/cli-auth?code=1#x')).toBe('app.beebeeb.io/cli-auth?code=1#x')
  })
})

describe('btnClass: one look for every button of the window', () => {
  test('plain, amber primary, red destructive; full width on request', () => {
    expect(btnClass('plain')).toBe('ms-btn')
    expect(btnClass('primary')).toBe('ms-btn ms-btn--primary')
    expect(btnClass('danger')).toBe('ms-btn ms-btn--danger')
    expect(btnClass('primary', true)).toBe('ms-btn ms-btn--primary mw-btn--full')
  })
})

// ── The router ──────────────────────────────────────────────────────────────

describe('SignInWindow: one screen per state (spec §7.4)', () => {
  const Stub = (name: string) => Object.defineProperty(function () { return null }, 'name', { value: name }) as (props: any) => null
  const stubs = { SignInScreen: Stub('SignInScreen'), SwitchScreen: Stub('SwitchScreen'), SetUpScreen: Stub('SetUpScreen'), FinderScreen: Stub('FinderScreen'), SettlingScreen: Stub('SettlingScreen') }
  // `moveFlow` (the one way a screen hands the flow on) tells Rust about a step that leaves the recovery phrase open.
  const told: Array<{ name: string; args: unknown }> = []
  const command = (name: string, args?: unknown) => { told.push({ name, args }); return Promise.resolve({ ok: true, value: undefined }) }
  const moveFlow = loadComponent('macSignIn.tsx', 'moveFlow', { command })
  const SignInWindow = loadComponent('macSignIn.tsx', 'SignInWindow', { React, ...stubs, ...model, Note: parts.Note, moveFlow, BACK_TO_HOME: copy.BACK_TO_HOME })
  const flows: model.SignInFlow[] = []
  const render = (props: Record<string, unknown>) => {
    flows.length = 0
    told.length = 0
    return SignInWindow({ email: 'sam@example.eu', flow: model.IDLE_FLOW, note: null, onFlow: (next: model.SignInFlow) => flows.push(next), ...props })
  }
  const screens = (tree: any) => elementsOf(tree).filter((el) => Object.values(stubs).includes(el.type as never))
  const nameOf = (el: TreeNode) => (el.type as { name: string }).name

  test('sign-in and sign-in-again are the same screen with a flag', () => {
    const signin = screens(render({ screen: 'signin' }))
    expect(signin.map(nameOf)).toEqual(['SignInScreen'])
    expect(signin[0].props.reauth).toBe(false)
    const again = screens(render({ screen: 'reauth' }))
    expect(again[0].props.reauth).toBe(true)
    expect(again[0].props.email).toBe('sam@example.eu')
  })

  test('after “Sign out and switch” the form opens with the address that was typed (FT-I4)', () => {
    const tree = render({ screen: 'signin', flow: { ...model.IDLE_FLOW, email: 'b@example.eu' } })
    expect(screens(tree)[0].props.prefillEmail).toBe('b@example.eu')
  })

  test('each of the other steps is its own screen', () => {
    const info = { pendingChanges: 2, unsent: null, unsentError: null }
    const sw = screens(render({ screen: 'switch', flow: { ...model.IDLE_FLOW, step: 'switch', switch: info } }))
    expect(sw.map(nameOf)).toEqual(['SwitchScreen'])
    expect(sw[0].props.info).toBe(info)
    const unlock = screens(render({ screen: 'unlock', flow: { ...model.IDLE_FLOW, step: 'unlock', keyReplaced: true } }))
    expect(unlock.map(nameOf)).toEqual(['SetUpScreen'])
    expect(unlock[0].props.keyReplaced).toBe(true)
    expect(screens(render({ screen: 'finder' })).map(nameOf)).toEqual(['FinderScreen'])
  })

  test('a sign-in that settled moves the flow on, with the address it was typed with', () => {
    const tree = render({ screen: 'signin' })
    screens(tree)[0].props.onSettled({ kind: 'fresh', vaultUnlocked: false }, 'sam@example.eu')
    expect(flows).toEqual([model.afterSignIn({ kind: 'fresh', vaultUnlocked: false }, 'sam@example.eu', false)])
  })

  test('in Sign in again a settled sign-in is read as the same account (the R8 rules)', () => {
    const tree = render({ screen: 'reauth' })
    screens(tree)[0].props.onSettled({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false }, 'sam@example.eu')
    expect(flows).toEqual([model.settling()])
  })

  // The case that depends on the flag: the browser path delivers the keys as a FRESH sign-in. At first sign-in that goes
  // on to the Finder screen; in Sign in again the account is already on this Mac, so it is done (Finder is kept).
  test('a browser sign-in is the Finder screen at first sign-in and "done" in Sign in again', () => {
    screens(render({ screen: 'signin' }))[0].props.onSettled({ kind: 'fresh', vaultUnlocked: true }, '')
    expect(flows).toEqual([model.afterSignIn({ kind: 'fresh', vaultUnlocked: true }, '', false)])
    expect(flows[0].step).toBe('finder')
    screens(render({ screen: 'reauth' }))[0].props.onSettled({ kind: 'fresh', vaultUnlocked: true }, '')
    expect(flows).toEqual([model.settling()])
  })

  test('the switch step hands back to sign-in with the typed address, or cancels to a clean sign-in', () => {
    const flow = { ...model.IDLE_FLOW, step: 'switch' as const, email: 'b@example.eu', switch: { pendingChanges: 1, unsent: null, unsentError: null } }
    const props = screens(render({ screen: 'switch', flow }))[0].props
    props.onSwitched()
    props.onCancel()
    expect(flows).toEqual([{ ...model.IDLE_FLOW, email: 'b@example.eu' }, model.IDLE_FLOW])
  })

  test('the recovery phrase leads to the Finder screen, and Finder ready to Home', () => {
    screens(render({ screen: 'unlock', flow: { ...model.IDLE_FLOW, step: 'unlock' } }))[0].props.onDone()
    expect(flows).toEqual([model.afterUnlocked()])
    screens(render({ screen: 'finder', flow: { ...model.IDLE_FLOW, step: 'finder' } }))[0].props.onReady()
    expect(flows).toEqual([model.afterFinderReady()])
  })

  test('a held sign-out sentence sits above the card, neutrally (spec §8.1)', () => {
    const tree = render({ screen: 'signin', note: 'Beebeeb couldn’t confirm it was removed from Finder.' })
    expect(textOf(tree)).toContain('Beebeeb couldn’t confirm it was removed from Finder.')
    const all = elementsOf(tree)
    const noteAt = all.findIndex((el) => el.type === parts.Note)
    const screenAt = all.findIndex((el) => el.type === stubs.SignInScreen)
    expect(all[noteAt].props.kind).toBe('status')
    expect(all.filter((el) => el.type === parts.Note && el.props.kind === 'alert')).toEqual([])
    expect(noteAt).toBeGreaterThanOrEqual(0)
    expect(noteAt).toBeLessThan(screenAt)
  })

  test('while the window waits for the read that follows a sign-in it shows only the settling screen, never the old card', () => {
    const tree = render({ screen: 'settling', flow: model.settling() })
    expect(screens(tree).map(nameOf)).toEqual(['SettlingScreen'])
    expect(screens(tree)[0].props.failed).toBe(false)
  })

  // Review P3-4: reads that keep failing give that screen the load-failed note and a way to read again.
  test('…and when the reads keep failing it says so, with a way to try again (P3-4)', () => {
    let retried = 0
    const tree = render({ screen: 'settling', flow: model.settling(), loadFailed: true, onRetryLoad: () => { retried += 1 } })
    const props = screens(tree)[0].props
    expect(props.failed).toBe(true)
    props.onRetry()
    expect(retried).toBe(1)
  })

  // Review P2-3: a sign-in that leaves the recovery phrase open (a password sign-in on a Mac with no key) is told to Rust,
  // which keeps the step for the life of the process: ⌘W destroys the window and the flow with it.
  test('a sign-in that leaves Set up this Mac open tells Rust, once; every other move tells nobody (P2-3)', () => {
    screens(render({ screen: 'signin' }))[0].props.onSettled({ kind: 'fresh', vaultUnlocked: false }, 'sam@example.eu')
    expect(flows.map((f) => f.step)).toEqual(['unlock'])
    expect(told).toEqual([{ name: 'set_pending_setup', args: { keyReplaced: false } }])
    screens(render({ screen: 'reauth' }))[0].props.onSettled({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: true }, 'sam@example.eu')
    expect(flows.map((f) => f.step)).toEqual(['unlock'])
    expect(told).toEqual([{ name: 'set_pending_setup', args: { keyReplaced: true } }])
    screens(render({ screen: 'signin' }))[0].props.onSettled({ kind: 'fresh', vaultUnlocked: true }, '')
    expect(told).toEqual([])
    screens(render({ screen: 'unlock', flow: { ...model.IDLE_FLOW, step: 'unlock' } }))[0].props.onDone()
    expect(told).toEqual([])
  })

  // Delta review P3-2: the requested “Sign in again” of a session that is fine has no Cancel; the shell hands the card a way
  // back when that is the case (`canLeaveSignInAgain`), and the card draws it as a quiet link under itself.
  test('a session that is fine reaches the card as such: the sentence follows the same signal as the way back (copy ruling)', () => {
    const fine = screens(render({ screen: 'reauth', onBack: () => {} }))[0].props
    expect(fine.sessionFine).toBe(true)
    expect(screens(render({ screen: 'reauth' }))[0].props.sessionFine).toBe(false)
    // The first sign-in is never “fine”, whatever handler the shell passes.
    expect(screens(render({ screen: 'signin', onBack: () => {} }))[0].props.sessionFine).toBe(false)
  })

  test('a way back is a quiet “Back to Home” under the Sign in again card, and nowhere else (P3-2)', () => {
    let left = 0
    const back = (tree: any) => elementsOf(tree).filter((el) => el.type === 'button' && textOf(el.props.children).trim() === 'Back to Home')
    const tree = render({ screen: 'reauth', onBack: () => { left += 1 } })
    expect(back(tree)).toHaveLength(1)
    expect(String(back(tree)[0].props.className)).toContain('mw-link--quiet')
    back(tree)[0].props.onClick()
    expect(left).toBe(1)
    expect(back(render({ screen: 'reauth' }))).toHaveLength(0)
    for (const screen of ['signin', 'switch', 'unlock', 'finder', 'settling'] as const) {
      expect(back(render({ screen, onBack: () => {} }))).toHaveLength(0)
    }
  })

  test('the offline state reaches the sign-in screen with its retry (P3-7)', () => {
    const offline = { onTryAgain: () => {} }
    expect(screens(render({ screen: 'signin', offline }))[0].props.offline).toBe(offline)
  })
})

describe('SettlingScreen: the window between a finished sign-in and the read that follows it (P3-4, P3-8)', () => {
  function open(props: Record<string, unknown>) {
    const m = mount('macSignIn.tsx', 'SettlingScreen', {
      backend: {},
      props: { failed: false, onRetry: () => {}, ...props },
      bindings: { ...copy, Note: parts.Note, Btn: parts.Btn, useWindowTitle: titleSpy },
      expand: true,
    })
    mounted.push(m)
    return m
  }

  test('it says only “Loading…” while the reads work', () => {
    const m = open({})
    expect(text(m)).toBe('Loading…')
    expect(m.elements().some((el) => el.props.role === 'status')).toBe(true)
    expect(buttonLabels(m)).toEqual([])
  })

  test('it carries the window’s title, “Beebeeb”: it is no card, and the last card’s heading would be stale (P3-8)', () => {
    open({})
    expect(windowTitles).toEqual(['Beebeeb'])
  })

  test('when the reads keep failing it shows the load-failed note and Try again, and Try again reads again (P3-4)', async () => {
    let retried = 0
    const m = open({ failed: true, onRetry: () => { retried += 1 } })
    expect(text(m)).toContain('Couldn’t load your account')
    expect(text(m)).not.toContain('Loading…')
    expect(m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    await button(m, 'Try again').props.onClick()
    expect(retried).toBe(1)
    expect(windowTitles).toEqual(['Beebeeb'])
  })
})

describe('SignInScreen: browser first, password second (C2)', () => {
  const Stub = (name: string) => Object.defineProperty(function () { return null }, 'name', { value: name }) as (props: any) => null
  const BrowserPanel = Stub('BrowserPanel')
  const PasswordPanel = Stub('PasswordPanel')
  const open = (props: Record<string, unknown> = {}) => {
    const m = mount('macSignIn.tsx', 'SignInScreen', {
      backend: {},
      props: { reauth: false, email: null, prefillEmail: undefined, offline: null, onSettled: () => {}, ...props },
      bindings: { BrowserPanel, PasswordPanel },
    })
    mounted.push(m)
    const shown = () => (m.elements().find((el) => el.type === BrowserPanel || el.type === PasswordPanel)!.type as { name: string }).name
    const panel = () => m.elements().find((el) => el.type === BrowserPanel || el.type === PasswordPanel)!
    return { m, shown, panel }
  }

  test('it opens on the browser', () => {
    expect(open().shown()).toBe('BrowserPanel')
  })

  test('“Use email and password” opens the form; “Sign in with browser instead” comes back', () => {
    const s = open()
    s.panel().props.onUsePassword()
    s.m.render()
    expect(s.shown()).toBe('PasswordPanel')
    s.panel().props.onUseBrowser()
    s.m.render()
    expect(s.shown()).toBe('BrowserPanel')
  })

  test('an address typed before a switch opens the form it was typed in, with that address as its first value', () => {
    const s = open({ prefillEmail: 'b@example.eu' })
    expect(s.shown()).toBe('PasswordPanel')
    expect(s.panel().props.prefillEmail).toBe('b@example.eu')
    // …and the way back to the browser is still there.
    s.panel().props.onUseBrowser()
    s.m.render()
    expect(s.shown()).toBe('BrowserPanel')
  })

  test('both methods hand a settled sign-in up through the same callback', () => {
    const settled: unknown[] = []
    const s = open({ onSettled: (result: unknown, email: unknown) => settled.push([result, email]) })
    s.panel().props.onSettled({ kind: 'fresh', vaultUnlocked: true }, '')
    s.panel().props.onUsePassword()
    s.m.render()
    s.panel().props.onSettled({ kind: 'fresh', vaultUnlocked: false }, 'sam@example.eu')
    expect(settled).toEqual([[{ kind: 'fresh', vaultUnlocked: true }, ''], [{ kind: 'fresh', vaultUnlocked: false }, 'sam@example.eu']])
  })
})

// ── The browser screen ──────────────────────────────────────────────────────

describe('BrowserPanel (hi-fi section 2, figures 1, 2 and 4; task 1734)', () => {
  const idle: any = { phase: 'idle', userCode: null, verificationUri: null, error: null }
  const Notice = () => null
  function open(over: { state?: any; props?: Record<string, unknown>; backend?: Record<string, (a: any) => unknown>; openUrl?: (url: string) => Promise<unknown>; write?: (t: Promise<string>) => Promise<void> } = {}) {
    const starts: string[] = []
    const opened: string[] = []
    const written: string[] = []
    const m = mount('macSignIn.tsx', 'BrowserPanel', {
      backend: { quit_beebeeb: () => null, ...over.backend },
      props: { reauth: false, email: null, offline: null, onSettled: () => {}, onUsePassword: () => starts.push('password'), ...over.props },
      bindings: {
        ...desktopApi,
        ...copy,
        ...browserCopy,
        SignInCard,
        btnClass,
        DiskImageNotice: Notice,
        Note: parts.Note,
        useWindowTitle: titleSpy,
        displayAddress,
        useBrowserSignIn: () => ({ state: over.state ?? idle, start: () => { starts.push('browser') } }),
        openUrl: async (url: string) => { opened.push(url); return over.openUrl ? over.openUrl(url) : { ok: true, value: undefined } },
        writeTextToPasteboard: async (value: Promise<string>) => { written.push(await value); await over.write?.(value) },
        FINDER_COPIED_MS: 2000,
      },
      expand: true,
    })
    mounted.push(m)
    return { m, starts, opened, written }
  }

  test('Sign in: the title, the amber button, the link, and the footer (spec C §7.1)', () => {
    const { m } = open()
    expect(textOf(heading(m)!.props.children)).toBe('Sign in to Beebeeb')
    expect(buttonLabels(m)).toEqual(['Sign in with browser', 'Use email and password', 'Quit Beebeeb'])
    expect(classes(button(m, 'Sign in with browser'))).toEqual(['ms-btn', 'ms-btn--primary', 'mw-btn--full'])
    expect(text(m)).toContain('New to Beebeeb?')
    expect(text(m)).toContain('Create an account on beebeeb.io')
    expect(text(m)).not.toContain('Signed in before')
  })

  test('the primary button starts the handoff; the link opens the form', async () => {
    const { m, starts } = open()
    await button(m, 'Sign in with browser').props.onClick()
    await button(m, 'Use email and password').props.onClick()
    expect(starts).toEqual(['browser', 'password'])
  })

  test('Sign in again (R8, C-R2): its title and sentence, the same two methods, the account named, no “Create account”', () => {
    const { m } = open({ props: { reauth: true, email: 'sam@example.eu' } })
    expect(textOf(heading(m)!.props.children)).toBe('Sign in again')
    expect(text(m)).toContain('Sync is paused until you sign in again. Your files stay in Finder.')
    expect(text(m)).toContain('Signed in before as sam@example.eu')
    expect(buttonLabels(m)).toEqual(['Sign in with browser', 'Use email and password', 'Quit Beebeeb'])
    expect(text(m)).not.toContain('Create an account')
    expect(text(m)).not.toContain('New to Beebeeb?')
  })

  // Lead ruling, 2026-10-25 (follow-up to #143): spec A's sentence says sync is paused, which is false when the session is
  // fine (the card was asked for after one operation answered 401, as Version Center does). That card says what happened.
  test('Sign in again on a session that is fine says what happened instead, and keeps the rest (copy ruling)', () => {
    const { m } = open({ props: { reauth: true, email: 'sam@example.eu', sessionFine: true } })
    expect(textOf(heading(m)!.props.children)).toBe('Sign in again')
    expect(text(m)).toContain('Beebeeb couldn’t send some changes with this sign-in. Sign in again to retry them.')
    expect(text(m)).not.toContain('Sync is paused')
    expect(text(m)).toContain('Signed in before as sam@example.eu')
    expect(buttonLabels(m)).toEqual(['Sign in with browser', 'Use email and password', 'Quit Beebeeb'])
  })

  test('the ended-session card keeps spec A’s sentence, and a first sign-in has none (copy ruling)', () => {
    const ended = open({ props: { reauth: true, email: 'sam@example.eu', sessionFine: false } })
    expect(text(ended.m)).toContain('Sync is paused until you sign in again. Your files stay in Finder.')
    expect(text(ended.m)).not.toContain('couldn’t send some changes')
    const first = open({ props: { reauth: false, email: null, sessionFine: true } })
    expect(text(first.m)).not.toContain('couldn’t send some changes')
    expect(text(first.m)).not.toContain('Sync is paused')
  })

  test('Sign in again never prints a missing address', () => {
    const { m } = open({ props: { reauth: true, email: null } })
    expect(text(m)).not.toContain('Signed in before')
    expect(text(m)).not.toContain('null')
  })

  test('connecting: the button says so and cannot be pressed twice', () => {
    const { m } = open({ state: { ...idle, phase: 'connecting' } })
    expect(button(m, 'Connecting…').props.disabled).toBe(true)
  })

  // Spec §10.1: amber is the one primary action of a view. While the handoff runs nothing can be pressed, so the
  // disabled button is the plain one (hi-fi figure 2), not a faded amber one.
  test('the button is amber only while it can be pressed: neutral while connecting, waiting and finishing', () => {
    for (const phase of ['connecting', 'waiting', 'authorized']) {
      const { m } = open({ state: { phase, userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null } })
      const label = { connecting: 'Connecting…', waiting: 'Waiting for browser…', authorized: 'Finishing…' }[phase]!
      expect({ phase, classes: classes(button(m, label)) }).toEqual({ phase, classes: ['ms-btn', 'mw-btn--full'] })
    }
    expect(classes(button(open().m, 'Sign in with browser'))).toEqual(['ms-btn', 'ms-btn--primary', 'mw-btn--full'])
    expect(classes(button(open({ state: { ...idle, phase: 'error', error: 'No.' } }).m, 'Try again'))).toEqual(['ms-btn', 'ms-btn--primary', 'mw-btn--full'])
  })

  test('waiting: the instruction, the code in mono, the address, “Copy link”, the disabled button and the way out (hi-fi figure 2)', () => {
    const { m } = open({ state: { phase: 'waiting', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null } })
    expect(text(m)).toContain(browserCopy.browserWaitingInstruction('waiting'))
    const code = m.elements().find((el) => el.props['aria-label'] === 'Sign-in code')!
    expect(textOf(code.props.children)).toBe('KQ7M-4TXP')
    expect(classes(code)).toContain('mw-code')
    expect(text(m)).toContain('Browser didn’t open? Visit')
    // Shown as the hi-fi draws it, without the scheme; "Copy link" copies the whole address.
    expect(text(m)).toContain('app.beebeeb.io/cli-auth')
    expect(text(m)).not.toContain('https://')
    expect(text(m)).toContain(browserCopy.BROWSER_DID_NOT_OPEN_HINT)
    expect(button(m, 'Waiting for browser…').props.disabled).toBe(true)
    expect(hasButton(m, 'Copy link')).toBe(true)
    expect(hasButton(m, 'Use email and password')).toBe(true)
  })

  test('“Copy link” puts the address on the pasteboard and says so for a moment', async () => {
    const { m, written } = open({ state: { phase: 'waiting', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null } })
    await button(m, 'Copy link').props.onClick()
    await m.flush()
    expect(written).toEqual(['https://app.beebeeb.io/cli-auth'])
    expect(hasButton(m, 'Copied')).toBe(true)
  })

  test('a refused pasteboard is a quiet failure: the address stays on screen to select', async () => {
    const { m } = open({
      state: { phase: 'waiting', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null },
      write: async () => { throw new Error('refused') },
    })
    await button(m, 'Copy link').props.onClick()
    await m.flush()
    expect(hasButton(m, 'Copied')).toBe(false)
    expect(text(m)).toContain('app.beebeeb.io/cli-auth')
    expect(m.toasts).toEqual([])
  })

  test('authorized: the finishing instruction, no address hint, the button says Finishing…', () => {
    const { m } = open({ state: { phase: 'authorized', userCode: 'KQ7M-4TXP', verificationUri: 'https://app.beebeeb.io/cli-auth', error: null } })
    expect(text(m)).toContain(browserCopy.browserWaitingInstruction('authorized'))
    expect(text(m)).not.toContain('Browser didn’t open?')
    expect(button(m, 'Finishing…').props.disabled).toBe(true)
  })

  test('an error is one alert, and the button offers “Try again”', async () => {
    const { m, starts } = open({ state: { ...idle, phase: 'error', error: 'Could not reach the server.' } })
    const alerts = m.elements().filter((el) => el.props.role === 'alert')
    expect(alerts).toHaveLength(1)
    expect(textOf(alerts[0].props.children)).toContain('Could not reach the server.')
    await button(m, 'Try again').props.onClick()
    expect(starts).toEqual(['browser'])
  })

  test('offline: the buttons are replaced by the sentence and “Try again”; Create account stays (spec C §7.1)', () => {
    const { m } = open({ props: { offline: { onTryAgain: () => {} } } })
    expect(text(m)).toContain('Can’t reach Beebeeb. Check your connection.')
    expect(hasButton(m, 'Sign in with browser')).toBe(false)
    expect(hasButton(m, 'Use email and password')).toBe(false)
    expect(hasButton(m, 'Try again')).toBe(true)
    expect(text(m)).toContain('Create an account on beebeeb.io')
  })

  // Review P3-7: offline, “Try again” re-checks the connection (the account view's retry); it never starts a handoff, which
  // would open the browser for a Mac that cannot reach Beebeeb.
  test('offline “Try again” re-checks connectivity and starts no sign-in (P3-7)', async () => {
    let rechecked = 0
    const { m, starts } = open({ props: { offline: { onTryAgain: () => { rechecked += 1 } } } })
    await button(m, 'Try again').props.onClick()
    expect(rechecked).toBe(1)
    expect(starts).toEqual([])
    expect(m.calls.map((c) => c.name)).not.toContain('start_browser_login')
  })

  test('“Create an account” is a real link to the address; clicking it opens the system browser, not this window', async () => {
    const { m, opened } = open()
    const link = m.elements().find((el) => el.type === 'a')!
    expect(link.props.href).toBe(desktopApi.CREATE_ACCOUNT_URL)
    expect(desktopApi.CREATE_ACCOUNT_URL).toBe('https://app.beebeeb.io/signup')
    let prevented = false
    await link.props.onClick({ preventDefault: () => { prevented = true } })
    expect(prevented).toBe(true)
    expect(opened).toEqual(['https://app.beebeeb.io/signup'])
  })

  test('when the browser will not open, the address is shown to select, with “Copy link” (spec C §10)', async () => {
    const { m } = open({ openUrl: async () => ({ ok: false, reason: 'no handler', unsupported: false }) })
    expect(text(m)).not.toContain('app.beebeeb.io/signup')
    await m.elements().find((el) => el.type === 'a')!.props.onClick({ preventDefault() {} })
    await m.flush()
    expect(text(m)).toContain('https://app.beebeeb.io/signup')
    expect(hasButton(m, 'Copy link')).toBe(true)
  })

  test('Quit Beebeeb quits (the same exit as ⌘Q)', async () => {
    const { m } = open()
    await button(m, 'Quit Beebeeb').props.onClick()
    expect(m.calls.map((c) => c.name)).toEqual(['quit_beebeeb'])
  })

  test('spec A’s “running from the disk image” notice sits above the buttons, and goes while the browser sign-in runs', () => {
    // The raw tree: the stub notice renders nothing, so the expanded tree would not show where it stood.
    const order = (m: Mounted) => elementsOf(m.tree()).filter((el) => el.type === Notice || el.type === 'button').map((el) => (el.type === Notice ? 'notice' : 'button'))
    expect(order(open().m)[0]).toBe('notice')
    expect(order(open({ state: { ...idle, phase: 'waiting', userCode: 'AAAA-BBBB' } }).m)).not.toContain('notice')
  })
})

describe('DiskImageNotice: spec A’s sentence and action, before sign-in (spec C §7.1)', () => {
  function open(controller: Record<string, unknown>) {
    const m = mount('macSignIn.tsx', 'DiskImageNotice', {
      backend: {},
      props: {},
      bindings: { Note: parts.Note, Btn: parts.Btn, finderActionButtonLabel: finderCopy.finderActionButtonLabel, useFinderSetup: () => controller },
      expand: true,
    })
    mounted.push(m)
    return m
  }
  const notice = (reason: string, over: Record<string, unknown> = {}) => ({
    presentation: { kind: 'notice', tone: 'alert', reason, sentence: finderCopy.FINDER_REASON_COPY[reason as keyof typeof finderCopy.FINDER_REASON_COPY].sentence, action: 'show_in_finder', actionLabel: 'Show in Finder' },
    run: async () => ({ ok: true, value: undefined }),
    copied: false,
    actionNote: null,
    ...over,
  })

  test('running from the disk image: the sentence and “Show in Finder”', async () => {
    const ran: string[] = []
    const m = open(notice('not_in_applications', { run: async (a: string) => { ran.push(a) } }))
    expect(text(m)).toContain('Beebeeb is running from the disk image. Move it to Applications, then open it again.')
    await button(m, 'Show in Finder').props.onClick()
    expect(ran).toEqual(['show_in_finder'])
  })

  test('any other Finder state says nothing before sign-in: it is not the sign-in screen’s business', () => {
    for (const reason of ['timeout', 'user_disabled', 'signing', 'unknown']) {
      expect(text(open(notice(reason)))).toBe('')
    }
    expect(text(open({ presentation: { kind: 'resting', line: 'x' }, run: async () => {}, copied: false, actionNote: null }))).toBe('')
  })
})

// ── The email form and the second factor ────────────────────────────────────

describe('PasswordPanel: today’s sign-in form, unchanged, in the window (spec §7.4 item 3)', () => {
  function open(backend: Record<string, (args: any) => unknown>, props: Record<string, unknown> = {}) {
    const done: unknown[] = []
    const emails: unknown[] = []
    const m = mount('macSignIn.tsx', 'PasswordPanel', {
      backend: { last_signed_in_email: () => null, ...backend },
      props: { prefillEmail: undefined, onUseBrowser: () => done.push('browser'), ...props, onSettled: (settled: unknown, email: unknown) => { done.push(settled); emails.push(email) } },
      bindings: {
        ...signIn,
        ...copy,
        lastSignedInEmail: desktopApi.lastSignedInEmail,
        SignInCard,
        btnClass,
        Note: parts.Note,
        useWindowTitle: titleSpy,
        requestAnimationFrame: () => 0,
      },
      expand: true,
    })
    mounted.push(m)
    return { m, done, emails }
  }
  const input = (m: Mounted, name: string) => m.elements().find((el) => el.type === 'input' && el.props.name === name)!
  const type = (m: Mounted, name: string, value: string) => { input(m, name).props.onChange({ currentTarget: { value } }); m.render() }
  const submit = async (m: Mounted) => { await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} }); await m.flush() }
  const plain = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null }

  test('the hi-fi’s card: “Welcome back”, the words, two fields, the amber button and the way to the browser', async () => {
    const { m } = open({})
    await m.flush()
    expect(textOf(heading(m)!.props.children)).toBe('Welcome back')
    expect(text(m)).toContain('Sign in to unlock your encrypted vault.')
    expect(input(m, 'email').props.type).toBe('email')
    expect(input(m, 'password').props.type).toBe('password')
    expect(buttonLabels(m)).toEqual(['Sign in', 'Sign in with browser instead'])
    const submitButton = m.elements().find((el) => el.type === 'button' && el.props.type === 'submit')!
    expect(textOf(submitButton.props.children)).toBe('Sign in')
    expect(classes(submitButton)).toEqual(['ms-btn', 'ms-btn--primary', 'mw-btn--full'])
    expect(submitButton.props.disabled).toBe(true)
  })

  test('the link goes back to the browser', async () => {
    const { m, done } = open({})
    await m.flush()
    await button(m, 'Sign in with browser instead').props.onClick()
    expect(done).toEqual(['browser'])
  })

  test('the address of the last account on this Mac is prefilled; an address typed before a switch beats it (FT-I4)', async () => {
    const prefilled = open({ last_signed_in_email: () => 'a@beebeeb.io' })
    await prefilled.m.flush(); await prefilled.m.flush()
    expect(input(prefilled.m, 'email').props.value).toBe('a@beebeeb.io')
    const switched = open({ last_signed_in_email: () => 'a@beebeeb.io' }, { prefillEmail: 'b@beebeeb.io' })
    await switched.m.flush(); await switched.m.flush()
    expect(input(switched.m, 'email').props.value).toBe('b@beebeeb.io')
  })

  test('a password sign-in hands its outcome and the typed address up; a fresh one asks for the recovery phrase next', async () => {
    const { m, done, emails } = open({ desktop_login: () => plain })
    await m.flush()
    type(m, 'email', 'user@beebeeb.io'); type(m, 'password', 'correct horse')
    await submit(m)
    expect(m.calls.filter((c) => c.name === 'desktop_login').map((c) => c.args)).toEqual([{ email: 'user@beebeeb.io', password: 'correct horse' }])
    expect(done).toEqual([{ kind: 'fresh', vaultUnlocked: false }])
    expect(emails).toEqual(['user@beebeeb.io'])
  })

  test('another account hands over the mismatch and its 1887 snapshot; nothing is signed out', async () => {
    const unsent = { askId: 'a1', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    const { m, done } = open({ desktop_login: () => ({ ...plain, account_mismatch: { pending_changes: 3, unsent } }) })
    await m.flush()
    type(m, 'email', 'user@beebeeb.io'); type(m, 'password', 'x')
    await submit(m)
    expect(done).toEqual([{ kind: 'account_mismatch', pendingChanges: 3, unsent, unsentError: null }])
    expect(m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('a failure is one alert on the form, verbatim, and the sign-in does not complete', async () => {
    const sentence = rustStr('lib.rs', 'SIGN_IN_KEY_NOT_REMOVED')
    const { m, done } = open({ desktop_login: () => { throw sentence } })
    await m.flush()
    type(m, 'email', 'user@beebeeb.io'); type(m, 'password', 'x')
    await submit(m)
    expect(done).toEqual([])
    const alerts = m.elements().filter((el) => el.props.role === 'alert')
    expect(alerts.map((el) => textOf(el.props.children))).toEqual([sentence])
  })

  test('a result it cannot read is an error, never a completed sign-in', async () => {
    const { m, done } = open({ desktop_login: () => ({ requires_2fa: false }) })
    await m.flush()
    type(m, 'email', 'user@beebeeb.io'); type(m, 'password', 'x')
    await submit(m)
    expect(done).toEqual([])
    expect(text(m)).toContain(switchCopy.SIGN_IN_OUTCOME_UNREADABLE)
  })

  test('while it signs in, the fields and the buttons are disabled', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => { release = resolve })
    const { m } = open({ desktop_login: () => gate.then(() => plain) })
    await m.flush()
    type(m, 'email', 'user@beebeeb.io'); type(m, 'password', 'x')
    void m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    m.render()
    expect(input(m, 'email').props.disabled).toBe(true)
    expect(input(m, 'password').props.disabled).toBe(true)
    expect(button(m, 'Sign in with browser instead').props.disabled).toBe(true)
    release()
    await m.flush()
  })

  describe('two-factor authentication (task 1521)', () => {
    const needs2fa = { ...plain, requires_2fa: true }
    async function toSecondFactor(backend: Record<string, (a: any) => unknown> = {}) {
      const s = open({ desktop_login: () => needs2fa, ...backend })
      await s.m.flush()
      type(s.m, 'email', 'user@beebeeb.io'); type(s.m, 'password', 'x')
      await submit(s.m)
      return s
    }

    test('a 2FA account does not complete at the password: the same card asks for the code', async () => {
      const { m, done } = await toSecondFactor()
      expect(done).toEqual([])
      expect(textOf(heading(m)!.props.children)).toBe('Two-factor authentication')
      expect(text(m)).toContain('Enter the 6-digit code from your authenticator app.')
      expect(input(m, 'code').props.inputMode).toBe('numeric')
      expect(input(m, 'code').props.autoComplete).toBe('one-time-code')
      expect(buttonLabels(m)).toEqual(['Verify', 'Use backup code', '← Back'])
    })

    test('only digits, six at most; Verify waits for all six', async () => {
      const { m } = await toSecondFactor()
      type(m, 'code', '12a4 56789')
      expect(input(m, 'code').props.value).toBe('124567')
      type(m, 'code', '12345')
      expect(m.elements().find((el) => el.type === 'button' && el.props.type === 'submit')!.props.disabled).toBe(true)
    })

    test('the code completes the sign-in and its outcome is the one handed up', async () => {
      const { m, done } = await toSecondFactor({ desktop_login_2fa: () => ({ ...plain, account_mismatch: { pending_changes: 2 } }) })
      type(m, 'code', '123456')
      await submit(m)
      expect(m.calls.filter((c) => c.name === 'desktop_login_2fa').map((c) => c.args)).toEqual([{ code: '123456' }])
      expect(done).toEqual([{ kind: 'account_mismatch', pendingChanges: 2, unsent: null, unsentError: null }])
    })

    test('a wrong code is retryable: the field clears, the card stays, one alert says why', async () => {
      const { m, done } = await toSecondFactor({ desktop_login_2fa: () => { throw new Error('That code is not right.') } })
      type(m, 'code', '123456')
      await submit(m)
      expect(done).toEqual([])
      expect(input(m, 'code').props.value).toBe('')
      expect(m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    })

    test('a backup code is eight digits and goes to the same command', async () => {
      const { m, done } = await toSecondFactor({ desktop_login_2fa: () => plain })
      await button(m, 'Use backup code').props.onClick()
      m.render()
      expect(text(m)).toContain('Enter one of the 8-digit backup codes you saved when enabling 2FA.')
      expect(hasButton(m, 'Use authenticator code instead')).toBe(true)
      type(m, 'code', '1234567890')
      expect(input(m, 'code').props.value).toBe('12345678')
      await submit(m)
      expect(m.calls.filter((c) => c.name === 'desktop_login_2fa').map((c) => c.args)).toEqual([{ code: '12345678' }])
      expect(done).toEqual([{ kind: 'fresh', vaultUnlocked: false }])
    })

    test('Back returns to the email form', async () => {
      const { m } = await toSecondFactor()
      await button(m, '← Back').props.onClick()
      m.render()
      expect(textOf(heading(m)!.props.children)).toBe('Welcome back')
    })
  })
})

// ── The account switch (task 1887, W11) ─────────────────────────────────────

describe('SwitchScreen: 1887’s account-switch step in the Beebeeb window (A0s to A4s, E3; spec §7.4 item 5, §8.2)', () => {
  const ASK = { askId: 'ask-1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] }

  /** A stand-in for the browser's focus. React attaches refs at commit, before effects run, so each button that
   *  carries a ref gets a fake element that records `focus()` BEFORE the first flush. */
  function withFocusModel(m: Mounted) {
    const focused: string[] = []
    for (const el of m.elements().filter((e) => e.type === 'button')) {
      const label = textOf(el.props.children).trim()
      if (el.props.ref && typeof el.props.ref === 'object') el.props.ref.current = { focus: () => focused.push(label) }
    }
    const pressEnter = async () => {
      const label = focused.at(-1)
      if (label === undefined) return
      await m.elements().find((e) => e.type === 'button' && textOf(e.props.children).trim() === label)!.props.onClick()
      await m.flush()
    }
    return { focused, pressEnter }
  }

  function openSwitch(info: Record<string, unknown>, clear: () => unknown = () => ({ warning: null }), extra: Record<string, (a: any) => unknown> = {}) {
    const events: string[] = []
    const m = mount('macSignIn.tsx', 'SwitchScreen', {
      backend: { clear_session: clear, ...extra },
      props: { info: { pendingChanges: 0, unsent: null, unsentError: null, ...info }, onSwitched: () => events.push('switched'), onCancel: () => events.push('cancelled') },
      bindings: { ...desktopApi, ...switchCopy, ...signOutCopy, ...accountSession, SignInCard, btnClass, useWindowTitle: titleSpy, Note: parts.Note },
      expand: true,
    })
    mounted.push(m)
    return { m, events }
  }
  const clearCalls = (m: Mounted) => m.calls.filter((c) => c.name === 'clear_session')

  test('Linux’s shape (no snapshot) and a count: says how many changes would be removed, in the drawn copy', async () => {
    const { m } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    expect(textOf(heading(m)!.props.children)).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(text(m)).toContain(switchCopy.accountSwitchBody(3))
  })

  test('nothing waiting names no loss; one change is singular', async () => {
    const none = openSwitch({ pendingChanges: 0 }); await none.m.flush()
    expect(text(none.m)).toContain(switchCopy.accountSwitchBody(0))
    const one = openSwitch({ pendingChanges: 1 }); await one.m.flush()
    expect(text(one.m)).toContain(switchCopy.accountSwitchBody(1))
  })

  test('with the ask snapshot (a Mac) the title and lines are A0s to A4s, and the folders listed are mono', async () => {
    // Folders are listed only when a saved copy is missing (A3e), so the snapshot has one.
    const withFolder = { ...ASK, missingFiles: 1, earlierFolders: ['/Users/sam/Changes not sent/one'] }
    const { m } = openSwitch({ unsent: withFolder })
    await m.flush()
    const lines = signOutCopy.askLines(withFolder, null, true)
    expect(lines.folders).toEqual(['/Users/sam/Changes not sent/one'])
    expect(textOf(heading(m)!.props.children)).toBe(signOutCopy.A0S_TITLE)
    expect(text(m)).toContain(signOutCopy.A1S_LINE)
    for (const line of lines.paragraphs) expect(text(m)).toContain(line)
    for (const folder of lines.folders) {
      expect(text(m)).toContain(folder)
      expect(classes(m.elements().find((el) => textOf(el.props.children) === folder)!)).toContain('mw-mono')
    }
  })

  test('E3: a count that could not be read is one line in the step, never a toast', async () => {
    const { m } = openSwitch({ unsentError: 'unsent_count_failed: the queue could not be read' })
    await m.flush()
    expect(text(m)).toContain('the queue could not be read')
    expect(text(m)).not.toContain('unsent_count_failed')
    expect(text(m)).toContain(signOutCopy.A1S_LINE)
    expect(m.toasts).toEqual([])
  })

  test('mounting it signs nothing out: the warning comes first', async () => {
    const { m, events } = openSwitch({ pendingChanges: 3 })
    await m.flush(); await m.flush()
    expect(m.calls).toEqual([])
    expect(events).toEqual([])
  })

  // Design §4 (lead ruling 2026-10-06): on open, focus is on Cancel, so Enter cancels and can never confirm.
  test('opens with focus on Cancel, and on nothing else', async () => {
    const { m } = openSwitch({ pendingChanges: 3 })
    const { focused } = withFocusModel(m)
    await m.flush()
    expect(focused).toEqual(['Cancel'])
  })

  test('Enter right after the step opens cancels; it never signs out', async () => {
    const { m, events } = openSwitch({ pendingChanges: 3 })
    const { pressEnter } = withFocusModel(m)
    await m.flush()
    await pressEnter()
    expect(events).toEqual(['cancelled'])
    expect(clearCalls(m)).toHaveLength(0)
  })

  test('Cancel changes nothing', async () => {
    const { m, events } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    await m.click('Cancel')
    expect(events).toEqual(['cancelled'])
    expect(m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('Sign out and switch signs out once, then hands back to sign-in', async () => {
    const { m, events } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    await m.click('Sign out and switch')
    expect(clearCalls(m)).toHaveLength(1)
    expect(events).toEqual(['switched'])
  })

  // FB-I2 (spec §5.6: "The switch leaves no vault key and no account email behind").
  test('the switch signs out with forgetEmail, so the previous address is not kept', async () => {
    const { m } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    await m.click('Sign out and switch')
    expect(clearCalls(m).map((c) => c.args)).toEqual([{ forgetEmail: true }])
  })

  test('with the ask snapshot the switch keeps the waiting saves with that snapshot’s askId (W11)', async () => {
    const { m } = openSwitch({ unsent: ASK })
    await m.flush()
    await m.click('Sign out and switch')
    expect(clearCalls(m).map((c) => c.args)).toEqual([{ forgetEmail: true, keepUnsent: { askId: 'ask-1' } }])
  })

  test('a refusal (unsent_changes) counts again and re-renders the ask, with no toast and no switch (W11)', async () => {
    const fresh = { askId: 'ask-2', files: 5, copiedFiles: 5, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }
    const { m, events } = openSwitch({ unsent: ASK }, () => { throw 'unsent_changes: 5 changes are waiting' }, { sign_out_count: () => fresh })
    await m.flush()
    await m.click('Sign out and switch')
    expect(events).toEqual([])
    expect(m.toasts).toEqual([])
    expect(text(m)).toContain(signOutCopy.askLines({ ...fresh }, null, true).paragraphs[0])
    // …and the next confirm carries the NEW snapshot.
    await m.click('Sign out and switch')
    expect(clearCalls(m).map((c) => c.args.keepUnsent)).toEqual([{ askId: 'ask-1' }, { askId: 'ask-2' }])
  })

  // Deviation from the shipped `AccountSwitchStep` (Linux's onboarding window, which sends no snapshot and so never
  // reaches this): there, an E3 that follows an ASK stayed hidden behind the stale ask, so the confirm did nothing and
  // said nothing. Here the stale ask goes and the line shows, which is what the spec says E3 is ("a line in the step"),
  // and the next confirm asks without a snapshot, so Rust's gate counts again before anything is signed out.
  test('a refusal whose recount fails is E3 as a line, never a toast, and the stale ask goes', async () => {
    const { m } = openSwitch({ unsent: ASK }, () => { throw 'unsent_changes: 2' }, { sign_out_count: () => { throw 'unsent_count_failed: the queue could not be read' } })
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.toasts).toEqual([])
    expect(text(m)).toContain('the queue could not be read')
    expect(text(m)).not.toContain('unsent_count_failed')
    expect(text(m)).not.toContain(signOutCopy.askLines(ASK, null, true).paragraphs[0])
    expect(textOf(heading(m)!.props.children)).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
  })

  test('the gate’s own count failing is E3 as a line, acknowledged once shown (F12); the next confirm carries no snapshot', async () => {
    const { m, events } = openSwitch({ unsent: ASK }, () => { throw 'unsent_count_failed: the queue could not be read' }, { sign_out_result_ack: () => null })
    await m.flush()
    await m.click('Sign out and switch')
    expect(events).toEqual([])
    expect(m.toasts).toEqual([])
    expect(text(m)).toContain('the queue could not be read')
    expect(m.calls.filter((c) => c.name === 'sign_out_result_ack')).toHaveLength(1)
    await m.click('Sign out and switch')
    expect(clearCalls(m).map((c) => c.args)).toEqual([{ forgetEmail: true, keepUnsent: { askId: 'ask-1' } }, { forgetEmail: true }])
  })

  // FB-24: a sign-out that happened but could not confirm Finder's removal is Ok with a warning. The sentence is HELD
  // (accountSession.ts), not toasted: this window sits inside AccountSessionBoundary's keyed tree, so the remount that
  // the sign-out causes took the toast with it (review P2-1). The signed-out card shows the held sentence.
  test('a sign-out that happened with a warning continues to sign-in, and holds the warning for the signed-out card (P2-1)', async () => {
    const sentence = rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING')
    const { m, events } = openSwitch({ pendingChanges: 2 }, () => ({ warning: { code: 'finder_removal_unconfirmed', sentence } }), { sign_out_result_ack: () => null })
    await m.flush()
    await m.click('Sign out and switch')
    expect(events).toEqual(['switched'])
    expect(accountSession.heldSignOutWarning()).toBe(sentence)
    expect(m.toasts).toEqual([])
    expect(m.calls.map((c) => c.name)).toContain('sign_out_result_ack')
  })

  test('a sign-out without a warning holds nothing, and clears what an earlier attempt held (P2-1)', async () => {
    accountSession.holdSignOutWarning('An earlier sentence.')
    const { m, events } = openSwitch({ pendingChanges: 2 }, () => ({ warning: null }))
    await m.flush()
    await m.click('Sign out and switch')
    expect(events).toEqual(['switched'])
    expect(accountSession.heldSignOutWarning()).toBeNull()
  })

  // Review I1 (lib.rs, clear_session): the alert comes first "also when the sign-out failed after the removal". By then the
  // account is signed out, the revision moves and this window is remounted: a toast would be gone with it. The step asks
  // the status once (`sync_status`: its answer is not guarded against a revision change) and holds the sentence when the
  // account is already out; with the account still here it is today's toast, and the step stays (below).
  test('a sign-out that failed AFTER the removal holds its sentence for the signed-out card, not a toast (P2-1)', async () => {
    const sentence = 'Sign-out stopped after Beebeeb was removed from Finder. Restart Beebeeb and sign in again.'
    const { m, events } = openSwitch({ pendingChanges: 2 }, () => { throw sentence }, { sign_out_result_ack: () => null, sync_status: () => ({ logged_in: false }) })
    await m.flush()
    await m.click('Sign out and switch')
    await m.flush()
    expect(accountSession.heldSignOutWarning()).toBe(sentence)
    expect(m.toasts).toEqual([])
    expect(events).toEqual([])
    const names = m.calls.map((c) => c.name)
    expect(names.indexOf('sign_out_result_ack')).toBeGreaterThan(names.indexOf('sync_status'))
  })

  test('a sign-out that failed with the account still signed in is a toast, nothing held (P2-1)', async () => {
    const { m } = openSwitch({ pendingChanges: 2 }, () => { throw new Error('engine busy') }, { sign_out_result_ack: () => null, sync_status: () => ({ logged_in: true }) })
    await m.flush()
    await m.click('Sign out and switch')
    await m.flush()
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(accountSession.heldSignOutWarning()).toBeNull()
  })

  // Delta review P3-5 (follow-up): when the status itself cannot be read the step cannot tell whether a remount is coming.
  // Losing the sentence is the worse mistake, so it is held; the toast is shown too, for the window that stays.
  test('a status that cannot be read holds the sentence AND shows the toast (P3-5)', async () => {
    const { m } = openSwitch({ pendingChanges: 2 }, () => { throw new Error('engine busy') }, { sign_out_result_ack: () => null, sync_status: () => { throw new Error('no status') } })
    await m.flush()
    await m.click('Sign out and switch')
    await m.flush()
    expect(accountSession.heldSignOutWarning()).toBe('engine busy')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(m.calls.filter((c) => c.name === 'sign_out_result_ack')).toHaveLength(1)
  })

  test('a status that does not say either way is the same: held and toasted (P3-5)', async () => {
    const { m } = openSwitch({ pendingChanges: 2 }, () => { throw new Error('engine busy') }, { sign_out_result_ack: () => null, sync_status: () => ({}) })
    await m.flush()
    await m.click('Sign out and switch')
    await m.flush()
    expect(accountSession.heldSignOutWarning()).toBe('engine busy')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
  })

  test('an address that could not be forgotten stops the switch: the Rust sentence verbatim, and the step stays', async () => {
    const sentence = rustStr('lib.rs', 'SIGN_OUT_EMAIL_NOT_FORGOTTEN')
    const { m, events } = openSwitch({ pendingChanges: 2 }, () => { throw sentence }, { sign_out_result_ack: () => null })
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.toasts.map((t) => t.message)).toEqual([sentence])
    expect(events).toEqual([])
    expect(textOf(heading(m)!.props.children)).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(m.elements().filter((el) => el.type === 'button').map((b) => b.props.disabled)).toEqual([false, false])
  })

  test('while the sign-out runs, neither button can be pressed again', async () => {
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => { release = resolve })
    const { m, events } = openSwitch({ pendingChanges: 3 }, () => gate.then(() => ({ warning: null })))
    await m.flush()
    await m.clickNoWait('Sign out and switch')
    const buttons = m.elements().filter((el) => el.type === 'button')
    expect(buttons.map((b) => [textOf(b.props.children).trim(), b.props.disabled])).toEqual([['Cancel', true], ['Sign out and switch', true]])
    release()
    await m.flush()
    expect(clearCalls(m)).toHaveLength(1)
    expect(events).toEqual(['switched'])
  })

  test('a failed sign-out is a toast, and the switch does not happen', async () => {
    const { m, events } = openSwitch({ pendingChanges: 1 }, () => { throw new Error('engine busy') }, { sign_out_result_ack: () => null })
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(m.toasts[0].message).toContain('engine busy')
    expect(events).toEqual([])
  })

  // signOutEntryPoints.test.tsx, F12: another error keeps today's toast, then acknowledges it once, after the sign-out ran.
  test('a failure is acknowledged once, after the sign-out that failed (F12)', async () => {
    const { m } = openSwitch({ unsent: ASK }, () => { throw new Error('Sign-out paused: x') }, { sign_out_result_ack: () => null })
    await m.flush()
    await m.click('Sign out and switch')
    await m.flush()
    const names = m.calls.map((c) => c.name)
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(names.filter((name) => name === 'sign_out_result_ack')).toHaveLength(1)
    expect(names.indexOf('sign_out_result_ack')).toBeGreaterThan(names.indexOf('clear_session'))
  })

  // signOutEntryPoints.test.tsx, §5.3 row 1 (plan review M8): a count that failed is E3, a line in the step, never a toast.
  // "Sign out and switch" then counts again through the gate; a re-count that fails is the same line, and one that
  // succeeds replaces the line with the ask.
  test('a count that failed shows E3 as a line, a failed re-count is the same line, and a fresh count replaces it', async () => {
    const e3 = `unsent_count_failed: ${signOutCopy.E3_UNSENT_COUNT_FAILED}`
    let counts = 0
    const { m } = openSwitch({ pendingChanges: 2, unsentError: e3 }, () => { throw new Error('unsent_changes: Some changes…') }, {
      sign_out_count: () => {
        counts += 1
        if (counts === 1) throw new Error(e3)
        return { askId: 'switch-3', files: 1, copiedFiles: 1, others: 1, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }
      },
    })
    await m.flush()
    expect(text(m)).toContain(signOutCopy.E3_UNSENT_COUNT_FAILED)
    expect(text(m)).not.toContain('unsent_count_failed')
    expect(textOf(heading(m)!.props.children)).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    await m.click('Sign out and switch')
    await m.flush()
    expect(m.toasts).toEqual([])
    expect(text(m)).toContain(signOutCopy.E3_UNSENT_COUNT_FAILED) // the re-count failed too: still the line
    await m.click('Sign out and switch')
    await m.flush()
    expect(m.toasts).toEqual([])
    expect(textOf(heading(m)!.props.children)).toBe(signOutCopy.A0S_TITLE)
    expect(text(m)).not.toContain(signOutCopy.E3_UNSENT_COUNT_FAILED)
    expect(clearCalls(m).map((c) => c.args)).toEqual([{ forgetEmail: true }, { forgetEmail: true }])
  })

  // The case the E3 line's clearing exists for: the fresh count succeeds and says NOTHING waits, so the ask is not shown
  // and only `countError` could still put the old line on screen.
  test('a fresh count that finds nothing waiting clears the E3 line: the plain step, never a stale failure', async () => {
    const e3 = `unsent_count_failed: ${signOutCopy.E3_UNSENT_COUNT_FAILED}`
    const empty = { askId: 'switch-4', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }
    const { m } = openSwitch({ pendingChanges: 2, unsentError: e3 }, () => { throw new Error('unsent_changes: Some changes…') }, { sign_out_count: () => empty })
    await m.flush()
    expect(text(m)).toContain(signOutCopy.E3_UNSENT_COUNT_FAILED)
    await m.click('Sign out and switch')
    await m.flush()
    expect(text(m)).not.toContain(signOutCopy.E3_UNSENT_COUNT_FAILED)
    expect(textOf(heading(m)!.props.children)).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(text(m)).toContain(switchCopy.accountSwitchBody(0))
  })

  test('the confirm is the red destructive button, never the amber one; Cancel is the plain button', async () => {
    const { m } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    expect(classes(button(m, 'Sign out and switch'))).toEqual(['ms-btn', 'ms-btn--danger'])
    expect(classes(button(m, 'Sign out and switch'))).not.toContain('ms-btn--primary')
    expect(classes(button(m, 'Cancel'))).toEqual(['ms-btn'])
  })

  test('Cancel comes first in the row, so the destructive choice is the second thing reached', async () => {
    const { m } = openSwitch({ pendingChanges: 3 })
    await m.flush()
    expect(buttonLabels(m)).toEqual(['Cancel', 'Sign out and switch'])
  })
})

// ── Set up this Mac ─────────────────────────────────────────────────────────

describe('SetUpScreen: the recovery phrase, only when this Mac has no key (spec §7.4 item 6)', () => {
  function open(props: Record<string, unknown> = {}, backend: Record<string, (a: any) => unknown> = {}) {
    const done: string[] = []
    const m = mount('macSignIn.tsx', 'SetUpScreen', {
      backend: { desktop_unlock_with_recovery_phrase: () => null, ...backend },
      props: { keyReplaced: false, onDone: () => done.push('done'), ...props },
      bindings: { ...desktopApi, ...copy, ...switchCopy, SignInCard, btnClass, Note: parts.Note, useWindowTitle: titleSpy, RECOVERY_WORD_COUNT: 12, requestAnimationFrame: () => 0 },
      expand: true,
    })
    mounted.push(m)
    return { m, done }
  }
  const words = (m: Mounted) => m.elements().filter((el) => el.type === 'input')
  const fill = (m: Mounted, list: string[]) => { list.forEach((w, i) => words(m)[i].props.onChange({ currentTarget: { value: w } })); m.render() }
  const PHRASE = 'orbit velvet canyon mirror harvest lantern ember quartz saddle tundra willow zephyr'.split(' ')

  test('the title, the words and the hint are the drawn ones', async () => {
    const { m } = open()
    await m.flush()
    expect(textOf(heading(m)!.props.children)).toBe('Set up this Mac')
    expect(text(m)).toContain('This Mac does not have your encryption keys yet. Restore them to continue.')
    expect(text(m)).toContain('Paste all 12 words into any box or type them one by one. Beebeeb stores the unlocked vault key in macOS Keychain for future unlocks.')
    expect(words(m)).toHaveLength(12)
    expect(words(m).map((el) => el.props['aria-label'])).toEqual(Array.from({ length: 12 }, (_, i) => `Recovery word ${i + 1}`))
  })

  test('after a same-account sign-in whose kept key changed, it opens with why it is asked (FB-I1)', async () => {
    const { m } = open({ keyReplaced: true })
    await m.flush()
    expect(text(m)).toContain('This account’s vault key was changed on another device. Enter your current recovery phrase to unlock.')
    expect(text(m)).not.toContain('does not have your encryption keys yet')
  })

  test('Unlock vault waits for all twelve words', async () => {
    const { m } = open()
    await m.flush()
    const unlock = () => m.elements().find((el) => el.type === 'button' && el.props.type === 'submit')!
    expect(unlock().props.disabled).toBe(true)
    fill(m, PHRASE.slice(0, 11))
    expect(unlock().props.disabled).toBe(true)
    fill(m, PHRASE)
    expect(unlock().props.disabled).toBe(false)
  })

  test('pasting all twelve words into any box fills the whole grid', async () => {
    const { m } = open()
    await m.flush()
    words(m)[5].props.onPaste({ preventDefault() {}, clipboardData: { getData: () => PHRASE.join('  \n') } })
    m.render()
    expect(words(m).map((el) => el.props.value)).toEqual(PHRASE)
  })

  test('a word with a space in it spreads over the boxes from there', async () => {
    const { m } = open()
    await m.flush()
    words(m)[2].props.onChange({ currentTarget: { value: 'canyon mirror harvest' } })
    m.render()
    expect(words(m).slice(2, 5).map((el) => el.props.value)).toEqual(['canyon', 'mirror', 'harvest'])
  })

  test('the phrase is sent once, as one string, and the step is done', async () => {
    const { m, done } = open()
    await m.flush()
    fill(m, PHRASE)
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'desktop_unlock_with_recovery_phrase').map((c) => c.args)).toEqual([{ recoveryPhrase: PHRASE.join(' ') }])
    expect(done).toEqual(['done'])
  })

  test('a wrong phrase is one alert with the reason, and the step stays with the words in place', async () => {
    const { m, done } = open({}, { desktop_unlock_with_recovery_phrase: () => { throw new Error('That recovery phrase does not match.') } })
    await m.flush()
    fill(m, PHRASE)
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
    expect(done).toEqual([])
    expect(m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    expect(text(m)).toContain('That recovery phrase does not match.')
    expect(words(m).map((el) => el.props.value)).toEqual(PHRASE)
  })

  test('typing again clears the alert', async () => {
    const { m } = open({}, { desktop_unlock_with_recovery_phrase: () => { throw new Error('No.') } })
    await m.flush()
    fill(m, PHRASE)
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
    words(m)[0].props.onChange({ currentTarget: { value: 'orbits' } })
    m.render()
    expect(m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(0)
  })

  test('a build without the unlock command says so and offers the way on (as the step always did)', async () => {
    const { m, done } = open({}, { desktop_unlock_with_recovery_phrase: () => { throw new Error('unknown command desktop_unlock_with_recovery_phrase') } })
    await m.flush()
    fill(m, PHRASE)
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
    expect(text(m)).toContain(desktopApi.commandUnavailableLabel('desktop_unlock_with_recovery_phrase'))
    await button(m, 'Continue to Finder setup').props.onClick()
    expect(done).toEqual(['done'])
  })

  test('there is no way on without a key otherwise: no “Continue” beside a working unlock', async () => {
    const { m } = open()
    await m.flush()
    expect(hasButton(m, 'Continue to Finder setup')).toBe(false)
  })

  test('the first word has focus when the step opens', async () => {
    const { m } = open()
    const focused: number[] = []
    words(m).forEach((el, i) => { if (el.props.ref && typeof el.props.ref === 'function') el.props.ref({ focus: () => focused.push(i) }) })
    await m.flush()
    expect(focused[0]).toBe(0)
  })
})

// ── Finder ──────────────────────────────────────────────────────────────────

describe('FinderScreen: a status screen, not a step (spec §7.5; spec A §6.2)', () => {
  function open(controller: Record<string, unknown>) {
    const ready: string[] = []
    const m = mount('macSignIn.tsx', 'FinderScreen', {
      backend: {},
      props: { onReady: () => ready.push('ready') },
      bindings: { ...copy, SignInCard, btnClass, Note: parts.Note, Btn: parts.Btn, useWindowTitle: titleSpy, finderActionButtonLabel: finderCopy.finderActionButtonLabel, useFinderSetup: () => controller },
      expand: true,
    })
    mounted.push(m)
    return { m, ready }
  }
  const quiet = { presentation: { kind: 'quiet', line: '' }, run: async () => ({ ok: true, value: undefined }), retry: async () => {}, copied: false, actionNote: null }

  test('the card: “Beebeeb in Finder”, “Beebeeb adds itself to Finder after sign-in.”, and nothing claimed before the reconciler speaks', async () => {
    const { m } = open(quiet)
    await m.flush()
    expect(textOf(heading(m)!.props.children)).toBe('Beebeeb in Finder')
    expect(text(m)).toContain('Beebeeb adds itself to Finder after sign-in.')
    expect(text(m)).not.toContain('Adding Beebeeb to Finder…')
    expect(buttonLabels(m)).toEqual(['Continue to Home'])
  })

  test('resting in Missing claims nothing either', async () => {
    const { m } = open({ ...quiet, presentation: { kind: 'resting', line: finderCopy.FINDER_RESTING_LINE } })
    await m.flush()
    expect(text(m)).not.toContain('Adding')
    expect(text(m)).not.toContain(finderCopy.FINDER_RESTING_LINE)
    expect(buttonLabels(m)).toEqual(['Continue to Home'])
  })

  test('adding: spec A’s line, as a status', async () => {
    const { m } = open({ ...quiet, presentation: { kind: 'adding', line: finderCopy.FINDER_ADDING_LINE } })
    await m.flush()
    const status = m.elements().find((el) => el.props.role === 'status' && textOf(el.props.children).includes('Adding Beebeeb to Finder…'))
    expect(status).toBeDefined()
  })

  // Review P3-6 (lead ruling, 2026-10-25), amending spec §7.5 (“‘Continue’ is not offered”): the card is a status screen,
  // and a Finder add that fails, or an extension that is off, must not hold a person on it. Home shows the Finder row.
  test('while Finder is not Ready the card offers “Continue to Home”, a secondary button, in every state (P3-6)', async () => {
    const states = [
      quiet.presentation,
      { kind: 'resting', line: finderCopy.FINDER_RESTING_LINE },
      { kind: 'adding', line: finderCopy.FINDER_ADDING_LINE },
      { kind: 'notice', tone: 'alert', reason: 'timeout', sentence: 'x', action: 'try_again', actionLabel: 'Try again' },
      { kind: 'notice', tone: 'alert', reason: 'unknown', sentence: 'y', action: null, actionLabel: null },
      { kind: 'unavailable', line: 'Beebeeb couldn’t read its Finder setup.', actionLabel: 'Try again' },
    ]
    for (const presentation of states) {
      const { m } = open({ ...quiet, presentation })
      await m.flush()
      expect(hasButton(m, 'Continue to Home')).toBe(true)
      expect(classes(button(m, 'Continue to Home'))).toContain('ms-btn')
      expect(classes(button(m, 'Continue to Home'))).not.toContain('ms-btn--primary')
    }
  })

  test('“Continue to Home” moves the window on once, and a Ready that arrives after it moves nothing again (P3-6)', async () => {
    const { m, ready } = open({ ...quiet, presentation: { kind: 'adding', line: finderCopy.FINDER_ADDING_LINE } })
    await m.flush()
    await button(m, 'Continue to Home').props.onClick()
    expect(ready).toEqual(['ready'])
    await button(m, 'Continue to Home').props.onClick()
    expect(ready).toEqual(['ready'])
  })

  test('the failure action stays the primary way forward: Try again is not displaced by Continue (P3-6)', async () => {
    const { m } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'alert', reason: 'timeout', sentence: finderCopy.FINDER_REASON_COPY.timeout.sentence, action: 'try_again', actionLabel: 'Try again' },
    })
    await m.flush()
    expect(buttonLabels(m)).toEqual(['Try again', 'Continue to Home'])
  })

  test('ready moves the window on to Home by itself, once; nothing is offered, and no “Continue”', async () => {
    const { m, ready } = open({ ...quiet, presentation: { kind: 'ready', line: finderCopy.FINDER_READY_LINE } })
    await m.flush(); await m.flush()
    expect(ready).toEqual(['ready'])
    expect(buttonLabels(m)).toEqual([])
  })

  test('a failure: spec A’s one sentence and one action, inline, as one alert (hi-fi “If it fails”)', async () => {
    const ran: string[] = []
    const { m, ready } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'alert', reason: 'timeout', sentence: finderCopy.FINDER_REASON_COPY.timeout.sentence, action: 'try_again', actionLabel: 'Try again' },
      run: async (a: string) => { ran.push(a) },
    })
    await m.flush()
    const alerts = m.elements().filter((el) => el.props.role === 'alert')
    expect(alerts).toHaveLength(1)
    expect(textOf(alerts[0].props.children)).toContain('macOS didn’t finish adding Beebeeb to Finder.')
    await button(m, 'Try again').props.onClick()
    expect(ran).toEqual(['try_again'])
    expect(ready).toEqual([])
  })

  // Spec §7.5 and the hi-fi: “On a failure or a turned-off extension: spec A's one sentence and one action, inline (ink on
  // --err-bg)”. The shipped Onboarding step drew “turned off” as a neutral status; the design wins, on this screen.
  test('turned off in System Settings is the same inline error as a failure, with its one action', async () => {
    const { m } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'status', reason: 'user_disabled', sentence: finderCopy.FINDER_REASON_COPY.user_disabled.sentence, action: 'open_system_settings', actionLabel: 'Open System Settings' },
    })
    await m.flush()
    expect(m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
    expect(text(m)).toContain('Beebeeb is turned off in System Settings.')
    expect(hasButton(m, 'Open System Settings')).toBe(true)
  })

  test('a relaunch-only refusal has a sentence and no button', async () => {
    const { m } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'alert', reason: 'unknown', sentence: 'Beebeeb’s sync didn’t confirm it stopped. Quit and reopen Beebeeb before syncing again.', action: null, actionLabel: null },
    })
    await m.flush()
    expect(buttonLabels(m)).toEqual(['Continue to Home'])
  })

  test('a note about a failed Try again is shown verbatim and neutrally', async () => {
    const { m } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'alert', reason: 'timeout', sentence: 'x', action: 'try_again', actionLabel: 'Try again' },
      actionNote: 'Beebeeb is not running yet. Open it again.',
    })
    await m.flush()
    expect(text(m)).toContain('Beebeeb is not running yet. Open it again.')
  })

  test('“Copied” replaces Copy details for a moment', async () => {
    const { m } = open({
      ...quiet,
      presentation: { kind: 'notice', tone: 'alert', reason: 'signing', sentence: 'x', action: 'copy_details', actionLabel: 'Copy details' },
      copied: true,
    })
    await m.flush()
    expect(hasButton(m, 'Copied')).toBe(true)
  })

  test('a state that cannot be read has a re-read, not the reconciler’s Try again', async () => {
    let retried = 0
    const { m } = open({ ...quiet, presentation: { kind: 'unavailable', line: finderCopy.FINDER_UNAVAILABLE_LINE, actionLabel: 'Try again' }, retry: async () => { retried += 1 } })
    await m.flush()
    expect(text(m)).toContain('Couldn’t check Finder.')
    await button(m, 'Try again').props.onClick()
    expect(retried).toBe(1)
  })

  test('the screen takes everything from useFinderSetup and keeps no Finder command of its own (ruling 7b)', () => {
    const body = readFileSync(new URL('../src/macSignIn.tsx', import.meta.url), 'utf8')
    const start = body.indexOf('export function FinderScreen(')
    const end = body.indexOf('\nexport function', start + 10)
    const screen = body.slice(start, end === -1 ? undefined : end)
    expect(screen.includes('useFinderSetup(')).toBe(true)
    for (const own of ['loadFinderSetup', 'subscribeFinderSetup', 'runFinderSetupAction', 'listen(', 'install_finder_location', 'continue_without_finder_location']) {
      expect(screen.includes(own)).toBe(false)
    }
  })
})

// ── The hooks ───────────────────────────────────────────────────────────────

describe('every card asks for the window title that is its own heading (spec C C-W8: “the title bar follows the heading”)', () => {
  const headingOf = (m: Mounted) => textOf(heading(m)!.props.children)
  const base = { onSettled: () => {}, onUsePassword: () => {}, onUseBrowser: () => {}, onSwitched: () => {}, onCancel: () => {}, onDone: () => {}, onReady: () => {} }
  const idleBrowser: any = { phase: 'idle', userCode: null, verificationUri: null, error: null }
  const finder = { presentation: { kind: 'quiet', line: '' }, run: async () => ({ ok: true, value: undefined }), retry: async () => {}, copied: false, actionNote: null }
  function titleOf(file: string, name: string, props: Record<string, unknown>, bindings: Record<string, unknown>, backend: Record<string, (a: any) => unknown> = {}) {
    const m = mount(file, name, {
      backend: { last_signed_in_email: () => null, ...backend },
      props: { ...base, ...props },
      bindings: { ...desktopApi, ...copy, ...switchCopy, ...signOutCopy, ...signIn, ...browserCopy, SignInCard, btnClass, displayAddress, Note: parts.Note, Btn: parts.Btn, useWindowTitle: titleSpy, requestAnimationFrame: () => 0, RECOVERY_WORD_COUNT: 12, finderActionButtonLabel: finderCopy.finderActionButtonLabel, DiskImageNotice: () => null, ...bindings },
      expand: true,
    })
    mounted.push(m)
    return m
  }

  test('sign in, sign in again, the email form, the second factor, the switch, set up this Mac and Finder', async () => {
    const browserBindings = { useBrowserSignIn: () => ({ state: idleBrowser, start: async () => {} }) }
    const cases: Array<[string, () => Mounted]> = [
      ['Sign in to Beebeeb', () => titleOf('macSignIn.tsx', 'BrowserPanel', { reauth: false, email: null, offline: null }, browserBindings)],
      ['Sign in again', () => titleOf('macSignIn.tsx', 'BrowserPanel', { reauth: true, email: 'sam@example.eu', offline: null }, browserBindings)],
      ['Welcome back', () => titleOf('macSignIn.tsx', 'PasswordPanel', { prefillEmail: undefined }, {})],
      ['Switch to a different account?', () => titleOf('macSignIn.tsx', 'SwitchScreen', { info: { pendingChanges: 1, unsent: null, unsentError: null } }, {})],
      ['Set up this Mac', () => titleOf('macSignIn.tsx', 'SetUpScreen', { keyReplaced: false }, {})],
      ['Beebeeb in Finder', () => titleOf('macSignIn.tsx', 'FinderScreen', {}, { useFinderSetup: () => finder })],
    ]
    for (const [expected, open] of cases) {
      windowTitles.length = 0
      const m = open()
      await m.flush()
      expect({ heading: headingOf(m), title: windowTitles.at(-1) }).toEqual({ heading: expected, title: expected })
    }
  })

  test('the second factor’s card is titled for what it asks, not for the email form it replaced', async () => {
    const m = titleOf('macSignIn.tsx', 'PasswordPanel', { prefillEmail: 'sam@example.eu' }, {}, {
      desktop_login: () => ({ requires_2fa: true, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null }),
    })
    await m.flush(); await m.flush()
    m.elements().find((el) => el.type === 'input' && el.props.name === 'password')!.props.onChange({ currentTarget: { value: 'x' } })
    m.render()
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
    expect(headingOf(m)).toBe('Two-factor authentication')
    expect(windowTitles.at(-1)).toBe('Two-factor authentication')
  })

  test('the switch step’s title follows its ask: the A0s title once a snapshot says something waits', async () => {
    const waiting = { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] }
    const m = titleOf('macSignIn.tsx', 'SwitchScreen', { info: { pendingChanges: 3, unsent: waiting, unsentError: null } }, {})
    await m.flush()
    expect({ heading: headingOf(m), title: windowTitles.at(-1) }).toEqual({ heading: signOutCopy.A0S_TITLE, title: signOutCopy.A0S_TITLE })
  })
})

describe('useBrowserSignIn: the controller as a hook (a new controller per mount, so StrictMode’s second mount is alive)', () => {
  function open() {
    const created: Array<{ started: number; detached: number; onState: (s: unknown) => void; onSettled: (s: unknown) => void }> = []
    class FakeBrowserSignIn {
      started = 0
      detached = 0
      constructor(_deps: unknown, public onState: (s: unknown) => void, public onSettled: (s: unknown) => void) { created.push(this) }
      async start() { this.started += 1 }
      detach() { this.detached += 1 }
    }
    const settled: unknown[] = []
    const idle = { phase: 'idle', userCode: null, verificationUri: null, error: null }
    const m = mount('browserSignIn.ts', 'useBrowserSignIn', {
      backend: {},
      props: { onSettled: (s: unknown) => settled.push(s) },
      bindings: { BrowserSignIn: FakeBrowserSignIn, IDLE_BROWSER_SIGN_IN: idle, browserSignInDeps: () => ({}) },
    })
    mounted.push(m)
    return { m, created, settled, idle }
  }

  test('it starts nothing by itself; its state is the idle one', async () => {
    const { m, created, idle } = open()
    await m.flush()
    expect(created).toHaveLength(1)
    expect(created[0].started).toBe(0)
    expect(m.tree().state).toEqual(idle)
  })

  test('start() starts the controller of this mount, once per call', async () => {
    const { m, created } = open()
    await m.flush()
    await m.tree().start()
    expect(created[0].started).toBe(1)
  })

  test('what the controller reports becomes the state, and what it settles is handed up', async () => {
    const { m, created, settled } = open()
    await m.flush()
    created[0].onState({ phase: 'waiting', userCode: 'AAAA-BBBB', verificationUri: null, error: null })
    m.render()
    expect(m.tree().state.phase).toBe('waiting')
    created[0].onSettled({ kind: 'fresh', vaultUnlocked: true })
    expect(settled).toEqual([{ kind: 'fresh', vaultUnlocked: true }])
  })

  // The attempt is not over when its card goes (see tests/browserSignIn.test.ts): the controller is detached, not stopped.
  test('leaving the screen detaches the controller; it does not stop the handoff', async () => {
    const { m, created } = open()
    await m.flush()
    m.unmount()
    expect(created[0].detached).toBe(1)
  })
})

describe('useWindowTitle: the window’s title follows the card’s heading (spec C C-W8, spec §10.2 row 36)', () => {
  test('it sets the title once for a heading, and again when the heading changes', async () => {
    const titles: string[] = []
    const m = mount('macSignIn.tsx', 'useWindowTitle', {
      backend: {},
      props: 'Sign in to Beebeeb',
      bindings: { setMainWindowTitle: async (title: string) => { titles.push(title) } },
    })
    mounted.push(m)
    await m.flush()
    expect(titles).toEqual(['Sign in to Beebeeb'])
    await m.flush()
    expect(titles).toEqual(['Sign in to Beebeeb'])
  })
})
