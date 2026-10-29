/**
 * Onboarding's Finder step, which is Linux's (task 1683 slice 2). Spec 2026-10-06 §10 gave macOS no install button; spec C
 * §11 then took macOS out of this window altogether: a Mac signs in in the Beebeeb window (`macSignIn.tsx`), and
 * `Onboarding.tsx` is the Linux first-run window (Windows has `WindowsFirstRun.tsx`). What used to be tested here for
 * macOS moved with the code: the Finder step and its use of `useFinderSetup` are `tests/macFinderScreen.test.tsx`
 * (every assertion carried over), the routing from the session is `tests/signInModel.test.ts`, and the rail wording for
 * macOS is gone with the rail's macOS variant.
 *
 * What stays is the guard those tests were written to hold ("on macOS no code path calls install_finder_location"),
 * stated now as what makes it true: this window never reads the platform or the Finder reconciler, so it cannot choose
 * the Mac's step, and a Mac never builds it (`surfaces::policy::onboarding_surface`, `main.tsx`'s HostOnboarding).
 *
 * What this proves: the component's real handlers and render output for scripted answers. What it does NOT prove:
 * layout, or Linux hardware ("Linux UI not checked on hardware", spec C §16).
 */
import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import ts from 'typescript'
import * as desktopApi from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import { elementsOf, loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const Wordmark = () => null
const Card = loadComponent('Onboarding.tsx', 'Card', { React, Wordmark })
const FORBIDDEN = ['install_finder_location', 'continue_without_finder_location', 'finder_location_state', 'finder_domain_user_enabled']

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const buttons = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())

describe('Onboarding source contract', () => {
  const source = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
  const ast = ts.createSourceFile('Onboarding.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const textOfFunction = (name: string) => {
    const declaration = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
    if (!declaration) throw new Error(`Missing function ${name} in Onboarding.tsx`)
    return declaration.getText(ast)
  }

  test('the install-era commands and the poll live only in the Windows/Linux step', () => {
    const rest = source.replace(textOfFunction('FinderInstallStep'), '')
    expect(FORBIDDEN.filter((name) => rest.includes(name))).toEqual([])
    const gone = ['USER_ENABLED_POLL_INTERVAL_MS', 'shouldRetryAfterUserEnabledPoll', 'finder_domain_user_enabled']
    expect(gone.filter((name) => source.includes(name))).toEqual([])
    expect(textOfFunction('FinderInstallStep').includes('desktop_platform')).toBe(false)
  })

  // Spec C §11: the macOS step, its routing by the reconciler, its platform fallback and its rail wording are removed.
  test('nothing of macOS is left in the window: no Mac step, no reconciler, no platform read, no capability read', () => {
    for (const gone of ['MacFinderStep', 'useFinderSetup', 'loadFinderSetup', 'FINDER_RAIL', 'FINDER_SETUP_TITLE', 'desktop_platform', 'useCapabilities', "'macos'"]) {
      expect({ gone, found: source.includes(gone) }).toEqual({ gone, found: false })
    }
    expect(() => textOfFunction('MacFinderStep')).toThrow()
  })

  test('a Mac never loads this window: HostOnboarding renders nothing for it, so the install-era step is unreachable there', () => {
    const main = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8')
    const host = main.slice(main.indexOf('function HostOnboarding()'), main.indexOf('let component: ReactElement'))
    expect(host).toContain("caps?.host_os === 'macos'")
    expect(host.indexOf("host_os === 'macos'")).toBeLessThan(host.indexOf('<Onboarding'))
    expect(host).toMatch(/host_os === 'macos'\) return null/)
  })
})

function stubs() {
  const make = (name: string) => Object.defineProperty(function () { return null }, 'name', { value: name }) as (props: any) => null
  return {
    SignInStep: make('SignInStep'),
    UnlockStep: make('UnlockStep'),
    FinderInstallStep: make('FinderInstallStep'),
    PinningStep: make('PinningStep'),
    ReadyStep: make('ReadyStep'),
  }
}

async function openOnboarding(backend: Record<string, (a: any) => unknown>, opts: { rail?: unknown[] } = {}) {
  const steps = stubs()
  const m = mount('Onboarding.tsx', 'OnboardingView', {
    backend,
    bindings: { ...desktopApi, ...steps, Wordmark, STEPS: opts.rail ?? [], useRegionLabel: () => 'the EU' },
  })
  mounted.push(m)
  await m.flush(); await tick(); await m.flush()
  const shown = () => Object.entries(steps).filter(([, type]) => m.elements().some((el) => el.type === type)).map(([name]) => name)
  const props = (name: keyof typeof steps) => m.elements().find((el) => el.type === steps[name])!.props
  return { m, shown, props }
}

const signedIn = (over: Partial<desktopApi.SyncStatus> = {}) => ({ logged_in: true, engine: 'idle', sync_root: null, vault_unlocked: true, syncing: 0, cloud_only: 0, conflicts: 0, ...over })
const signedOut = { logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }

describe('Onboarding routes by the sync folder (Linux)', () => {
  test('keys present: a sync folder decides. None: the Finder step; one: the last step', async () => {
    const without = await openOnboarding({ sync_status: () => signedIn() })
    expect(without.shown()).toEqual(['FinderInstallStep'])
    const withRoot = await openOnboarding({ sync_status: () => signedIn({ sync_root: '/home/sam/Beebeeb' }) })
    expect(withRoot.shown()).toEqual(['ReadyStep'])
  })

  test('vault still locked: the unlock step comes first', async () => {
    const o = await openOnboarding({ sync_status: () => signedIn({ vault_unlocked: false }) })
    expect(o.shown()).toEqual(['UnlockStep'])
  })

  test('signing in and unlocking lead to the Finder install step, which leads on to the offline folders', async () => {
    const o = await openOnboarding({ sync_status: () => signedOut })
    expect(o.shown()).toEqual(['SignInStep'])
    o.props('SignInStep').onDone({ kind: 'fresh', vaultUnlocked: false }); o.m.render()
    expect(o.shown()).toEqual(['UnlockStep'])
    o.props('UnlockStep').onDone(); o.m.render()
    expect(o.shown()).toEqual(['FinderInstallStep'])
    o.props('FinderInstallStep').onDone(); o.m.render()
    expect(o.shown()).toEqual(['PinningStep'])
  })

  // The guard those tests held, as what makes it true: whatever the platform says, this window never asks.
  test('it never reads the platform or the Finder reconciler, so no answer from either can choose another step', async () => {
    for (const status of [signedIn(), signedIn({ sync_root: '/home/sam/Beebeeb' }), signedIn({ vault_unlocked: false }), signedOut]) {
      const o = await openOnboarding({ sync_status: () => status, desktop_platform: () => 'macos', finder_setup_state: () => ({ setup: 'ready' }) })
      const names = o.m.calls.map((c) => c.name)
      expect(names).not.toContain('desktop_platform')
      expect(names).not.toContain('finder_setup_state')
      expect(names.filter((name) => FORBIDDEN.includes(name))).toEqual([])
    }
  })
})

describe('Onboarding step rail: the same five rows on every platform that mounts it', () => {
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

  test('the rail the source declares is exactly today’s', () => {
    expect(realSteps().map((row: any) => [row.title, row.detail])).toEqual(unchanged)
  })

  test('the rail is drawn as declared, whatever the platform says: there is no variant for any platform any more', async () => {
    for (const platform of ['linux', 'windows', 'unknown', 'macos']) {
      const o = await openOnboarding({ sync_status: () => signedOut, desktop_platform: () => platform }, { rail: realSteps() })
      expect(rail(o.m)).toEqual(unchanged)
    }
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
