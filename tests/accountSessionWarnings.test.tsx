/**
 * FB-24 / must-render rows 12 and 13 (M4) on the two account surfaces outside the macOS Settings
 * window: the compact Account page (`pages/Account.tsx`, the Linux lock surface, also the macOS compact
 * window) and the main window's Disconnect (`windows/views/AccountView.tsx`, DisconnectSection).
 *
 * A Lock or a sign-out that HAPPENED but could not confirm a step is Ok with a warning. Its sentence is
 * shown neutrally, never under "Couldn’t …"; an Err still means it did not happen; and the page reads
 * the status again after ANY result, so it never keeps claiming "Unlocked" after a Lock that ran.
 *
 * The real component declarations run with the component harness against a scripted backend.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as diagnosticsCopy from '../src/diagnosticsCopy'
import * as planPresentation from '../src/planPresentation'
import { mount, textOf, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const ENGINE_WARNING = { code: 'engine_stop_unconfirmed', sentence: rustStr('lib.rs', 'LOCK_ENGINE_UNCONFIRMED_WARNING') }
const SIGN_OUT_WARNING = { code: 'finder_removal_unconfirmed', sentence: rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING') }

type Status = { unlocked: boolean; loggedIn: boolean }

function openAccountPage(over: (st: Status) => Record<string, (a: any) => unknown> = () => ({})) {
  const st: Status = { unlocked: true, loggedIn: true }
  const m = mount('pages/Account.tsx', 'Account', {
    backend: {
      sync_status: () => ({ logged_in: st.loggedIn, vault_unlocked: st.unlocked, engine: st.unlocked ? 'running' : 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
      autostart_enabled: () => false,
      account_email: () => 'sam@beebeeb.io',
      account_subscription: () => { throw new Error('not in this test') },
      ...over(st),
    },
    bindings: { ...desktopApi, ...diagnosticsCopy, ...planPresentation, WEB_APP_URL: 'https://app.beebeeb.io', useRegionLabel: () => 'Stored in the EU' },
  })
  mounted.push(m)
  const settle = async () => { for (let i = 0; i < 4; i++) await m.flush() }
  const statusReads = () => m.calls.filter((c) => c.name === 'sync_status').length
  const notes = () => m.elements().filter((el) => el.props.role === 'status').map((el) => textOf(el.props.children).trim())
  return { m, st, settle, statusReads, notes }
}

describe('Account page (Linux lock surface): a Lock or sign-out that happened with a warning', () => {
  test('a Lock with a warning: the page reads the status again and says the warning in a neutral line, no toast', async () => {
    const page = openAccountPage((st) => ({ lock_vault: () => { st.unlocked = false; return { warning: ENGINE_WARNING } } }))
    await page.settle()
    const before = page.statusReads()
    await page.m.click('Lock')
    await page.settle()
    expect(page.statusReads()).toBeGreaterThan(before)
    expect(page.notes()).toContain(ENGINE_WARNING.sentence)
    expect(page.m.toasts).toEqual([])
    expect(visibleErrorSurfaces(page.m)).toEqual([])
    expect(textOf(page.m.tree())).toContain('Locked or paused')
  })

  test('a failed Lock is one error toast, and the page still reads the status again (M4)', async () => {
    const page = openAccountPage(() => ({ lock_vault: () => { throw new Error('Could not stop the sync engine') } }))
    await page.settle()
    const before = page.statusReads()
    await page.m.click('Lock')
    await page.settle()
    expect(page.m.toasts.map((t) => t.variant)).toEqual(['error'])
    expect(page.statusReads()).toBeGreaterThan(before)
  })

  test('a sign-out with a warning: signed out, and the warning in a neutral line, never "Couldn’t sign out"', async () => {
    const page = openAccountPage((st) => ({ clear_session: () => { st.loggedIn = false; return { warning: SIGN_OUT_WARNING } } }))
    await page.settle()
    await page.m.click('Sign out')
    await page.settle()
    expect(page.notes()).toContain(SIGN_OUT_WARNING.sentence)
    expect(page.m.toasts).toEqual([])
    expect(textOf(page.m.tree())).toContain('Signed out')
  })
})

describe('main window Disconnect: a sign-out that happened with a warning', () => {
  function openDisconnect(clear: () => unknown) {
    const m = mount('windows/views/AccountView.tsx', 'DisconnectSection', {
      backend: { clear_session: clear },
      bindings: { ...desktopApi, T: new Proxy({}, { get: () => '' }), DANGER: 'red' },
    })
    mounted.push(m)
    return m
  }

  test('the warning is a neutral note that outlives this view (it unmounts on sign-out), never an error', async () => {
    const m = openDisconnect(() => ({ warning: SIGN_OUT_WARNING }))
    await m.flush()
    await m.click('Disconnect')
    await m.click('Disconnect')
    expect(m.toasts.map((t) => ({ variant: t.variant, title: t.title, message: t.message }))).toEqual([{ variant: 'info', title: undefined, message: SIGN_OUT_WARNING.sentence }])
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('no warning, no note', async () => {
    const m = openDisconnect(() => ({ warning: null }))
    await m.flush()
    await m.click('Disconnect')
    await m.click('Disconnect')
    expect(m.toasts).toEqual([])
  })
})
