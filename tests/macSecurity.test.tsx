/**
 * Settings › Security (task 1683 slice 5; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8, row "Security";
 * hi-fi section 4, "Security (new)"): the real `SecurityTab` of `src/MacSecurity.tsx`, executed with the component harness
 * against a scripted backend.
 *
 * What this proves: the four reads it makes and no others, the words a person reads, "nothing destructive without a
 * second click", one failure with one surface (a one-off action is a toast, a group that did not load is one inline
 * alert with Try again, and one group failing leaves the others alone), and that a locked vault shows no account data.
 * What it does NOT prove: layout (tests/render-mac-app.mjs measures that in Chromium) or anything on a Mac.
 */
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as parts from '../src/macSettingsParts'
import * as securityModel from '../src/macSecurityModel'
import { expand, loadComponent, mount, textOf, visibleErrorSurfaces, type Mounted, type TreeNode } from './fixtures/componentHarness'

const React = { createElement, Fragment }

/** The in-window dialog, reduced to what a user sees: nothing while closed, title, body and footer while open. */
function Modal(props: any) {
  return props.open
    ? createElement('div', { role: 'dialog', 'aria-label': props.title }, createElement('h2', null, props.title), props.children, props.footer)
    : null
}
const ConfirmSheet = loadComponent('MacSettings.tsx', 'ConfirmSheet', { React, Btn: parts.Btn, Modal })

const mounted: Mounted[] = []
afterEach(() => {
  focused.length = 0
  while (mounted.length) mounted.pop()!.close()
})

const hoursAgo = (n: number) => new Date(Date.now() - n * 3_600_000).toISOString()
const today = (h: number, m: number) => { const d = new Date(); d.setHours(h, m, 0, 0); return d }
const hhmm = (d: Date) => `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`

const profile = (totp: boolean | null) => ({ user_id: 'u1', email: 'sam@example.eu', email_verified: true, created_at: hoursAgo(5000), totp_enabled: totp })
const session = (id: string, name: string, over: object = {}) => ({ id, device_name: name, device_kind: 'desktop', country_code: 'NL', last_active_at: hoursAgo(2), created_at: hoursAgo(500), is_current: false, ...over })
const sessionsOf = (...list: object[]) => ({ sessions: list })
const mac = session('s1', 'MacBook Pro', { is_current: true, last_active_at: new Date().toISOString() })
const phone = session('s2', 'iPhone', { device_kind: 'ios' })
const ipad = session('s3', 'iPad', { device_kind: 'ios', last_active_at: hoursAgo(30) })
const device = (over: object = {}) => ({ id: 'd1', hostname: 'MacBook Pro', platform: 'macos', bb_version: '0.8.13', last_seen: new Date().toISOString(), created_at: hoursAgo(24 * 60), session_count: 0, ...over })
const trashed = today(9, 12)
const feed = (events: object[]) => ({ events, total: events.length, page: 1 })
const trashEvent = { id: 'e1', type: 'file.trash', subject: null, details: null, where: null, created_at: trashed.toISOString() }

function backendOf(over: Record<string, (args: any) => unknown> = {}) {
  return {
    account_profile_fresh: () => profile(true),
    account_session_list: () => sessionsOf(mac, phone),
    account_devices: () => ({ devices: [device()] }),
    account_activity_feed: () => feed([trashEvent]),
    account_revoke_session: () => undefined,
    account_revoke_other_sessions: () => ({ revoked: 1 }),
    'plugin:opener|open_url': () => undefined,
    ...over,
  }
}

/** What the page moved focus to, in order (`focusById` needs a document; the id is the whole story). */
const focused: string[] = []

function openSecurity(backend: Record<string, (args: any) => unknown> = backendOf(), props: object = { unlocked: true }) {
  const m = mount('MacSecurity.tsx', 'SecurityTab', {
    backend: backend as any,
    expand: true,
    props,
    bindings: { ...desktopApi, ...securityModel, ...parts, ConfirmSheet, focusById: (id: string) => { focused.push(id) } },
  })
  mounted.push(m)
  return m
}

const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
const buttons = (m: Mounted) => find(m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim()).filter(Boolean)
const byLabel = (m: Mounted, label: string) => {
  const hit = find(m, (el) => el.type === 'button' && (el.props['aria-label'] === label || textOf(el.props.children).trim() === label))
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}; buttons: ${buttons(m).join(' | ')}`)
  return hit[0]
}
const press = async (m: Mounted, label: string) => { await byLabel(m, label).props.onClick(); await settle(m) }
function readable(node: any): string[] {
  if (Array.isArray(node)) return node.flatMap(readable)
  if (node == null || typeof node === 'boolean') return []
  if (typeof node !== 'object' || !node.props) return [String(node)]
  if (node.type === 'svg') return []
  return readable(node.props.children)
}
const visibleText = (m: Mounted) => readable(expand(m.tree())).join(' | ')
const dialogs = (m: Mounted) => find(m, (el) => el.props.role === 'dialog')
const alerts = (m: Mounted) => find(m, (el) => el.props.role === 'alert')
const called = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name)

describe('Security: what it reads and shows', () => {
  test('reads the profile (for the authenticator), the sessions, the devices and the last ten events once each, and nothing else', async () => {
    const m = openSecurity()
    await settle(m)
    expect(m.calls.map((c) => c.name).sort()).toEqual(['account_activity_feed', 'account_devices', 'account_profile_fresh', 'account_session_list'])
    expect(called(m, 'account_activity_feed')[0].args).toEqual({ page: 1, limit: 10 })
  })

  test('shows every group the hi-fi draws, in its order, in the Windows shell\'s words', async () => {
    const m = openSecurity()
    await settle(m)
    const text = visibleText(m)
    const order = [
      'Authenticator app (TOTP) · On',
      'A second factor is required when you sign in. Manage or reset it on beebeeb.io.',
      'Active sessions',
      'MacBook Pro · This session', 'Active now',
      'iPhone', 'Active 2 hours ago',
      'Sign out everywhere else', 'Ends every session except this one. Those devices will need to sign in again.',
      'Devices',
      'MacBook Pro · macOS · v0.8.13', 'last seen just now · registered',
      'Every device holding a copy of your vault key, and the sync sessions running on each.',
      'Recent account activity',
      'Moved an item to trash', `Today ${hhmm(trashed)}`,
    ]
    let from = 0
    for (const words of order) {
      const at = text.indexOf(words, from)
      expect(at, `"${words}" after position ${from} in: ${text}`).toBeGreaterThanOrEqual(0)
      from = at + words.length
    }
    expect(text).not.toMatch(/revoke a device|re-encrypt/i)
  })

  test('the buttons: the authenticator link, a Revoke for each other session (never this one), and sign out everywhere else', async () => {
    const m = openSecurity()
    await settle(m)
    expect(buttons(m)).toEqual(['Manage on beebeeb.io', 'Revoke', 'Sign out all other sessions'])
    expect(find(m, (el) => el.type === 'button' && /^Revoke /.test(String(el.props['aria-label']))).map((el) => el.props['aria-label'])).toEqual(['Revoke iPhone'])
  })

  test('with no other session there is nothing to revoke and no "everywhere else" row', async () => {
    const m = openSecurity(backendOf({ account_session_list: () => sessionsOf(mac) }))
    await settle(m)
    expect(buttons(m)).toEqual(['Manage on beebeeb.io'])
    expect(visibleText(m)).not.toContain('Sign out everywhere else')
  })

  test('the authenticator link opens the security page on the web app; off says to set it up', async () => {
    const m = openSecurity()
    await settle(m)
    await press(m, 'Manage on beebeeb.io')
    expect(called(m, 'plugin:opener|open_url')[0].args).toEqual({ url: securityModel.SECURITY_SETTINGS_URL })

    const off = openSecurity(backendOf({ account_profile_fresh: () => profile(false) }))
    await settle(off)
    expect(visibleText(off)).toContain('Authenticator app (TOTP) · Off')
    expect(visibleText(off)).toContain('Add a second factor so a stolen password alone can’t open your vault. Set it up on beebeeb.io.')
    expect(buttons(off)[0]).toBe('Set up on beebeeb.io')
  })

  test('devices count the sync sessions on the row, and ask for no second list', async () => {
    const m = openSecurity(backendOf({ account_devices: () => ({ devices: [device({ session_count: 2 })] }) }))
    await settle(m)
    expect(visibleText(m)).toContain('2 sync sessions')
    expect(called(m, 'account_client_sessions')).toHaveLength(0)
  })

  test('empty groups say so in a plain row', async () => {
    const m = openSecurity(backendOf({ account_session_list: () => sessionsOf(), account_devices: () => ({ devices: [] }), account_activity_feed: () => feed([]) }))
    await settle(m)
    const text = visibleText(m)
    expect(text).toContain('No active sessions')
    expect(text).toContain('No devices registered yet')
    expect(text).toContain('No recent activity.')
    expect(alerts(m)).toEqual([])
  })

  test('a locked vault shows no account data and asks for none', async () => {
    const m = openSecurity(backendOf(), { unlocked: false })
    await settle(m)
    expect(m.calls).toEqual([])
    const text = visibleText(m)
    expect(text).toContain('Your vault is locked')
    expect(text).toContain('Unlock it to see your sessions, devices and recent activity.')
    expect(text).not.toContain('Active sessions')
  })
})

describe('Security: revoking is asked first, and fails in one place', () => {
  test('Revoke opens a question and sends nothing; Cancel sends nothing', async () => {
    const m = openSecurity()
    await settle(m)
    await press(m, 'Revoke iPhone')
    expect(called(m, 'account_revoke_session')).toHaveLength(0)
    expect(dialogs(m)).toHaveLength(1)
    expect(visibleText(m)).toContain('Sign out this session?')
    expect(visibleText(m)).toContain('iPhone will need to sign in again.')
    await press(m, 'Cancel')
    expect(called(m, 'account_revoke_session')).toHaveLength(0)
    expect(dialogs(m)).toHaveLength(0)
  })

  test('confirming revokes exactly that session, reads the sessions and the activity again, and says so once', async () => {
    let revoked = false
    const m = openSecurity(backendOf({
      account_session_list: () => sessionsOf(...(revoked ? [mac] : [mac, phone])),
      account_revoke_session: () => { revoked = true },
    }))
    await settle(m)
    await press(m, 'Revoke iPhone')
    await press(m, 'Sign out')
    expect(called(m, 'account_revoke_session').map((c) => c.args)).toEqual([{ sessionId: 's2' }])
    expect(called(m, 'account_session_list')).toHaveLength(2)
    expect(called(m, 'account_activity_feed')).toHaveLength(2)
    expect(called(m, 'account_devices')).toHaveLength(1)
    expect(dialogs(m)).toHaveLength(0)
    expect(visibleText(m)).not.toContain('iPhone')
    expect(m.toasts).toEqual([expect.objectContaining({ variant: 'success', title: 'Session signed out' })])
  })

  // PR #140 review, P3: the Revoke button that opened the question is gone once its row is, and focus fell to the page.
  test('after a revoke the row that had focus is gone, so focus goes to the "Active sessions" heading', async () => {
    let revoked = false
    const m = openSecurity(backendOf({
      account_session_list: () => sessionsOf(...(revoked ? [mac] : [mac, phone])),
      account_revoke_session: () => { revoked = true },
    }))
    await settle(m)
    const heading = find(m, (el) => el.type === 'h2' && textOf(el.props.children) === 'Active sessions')
    expect(heading).toHaveLength(1)
    expect(heading[0].props.id).toBe(securityModel.SESSIONS_HEADING_ID)
    expect(heading[0].props.tabIndex).toBe(-1)
    expect(focused).toEqual([])
    await press(m, 'Revoke iPhone')
    expect(focused, 'asking the question moves nothing: the sheet takes focus and gives it back itself').toEqual([])
    await press(m, 'Sign out')
    await settle(m)
    expect(focused).toEqual([securityModel.SESSIONS_HEADING_ID])
  })

  test('after "Sign out all other sessions" the button is gone too, and focus goes to the same heading', async () => {
    const m = openSecurity(backendOf({ account_session_list: () => sessionsOf(mac, phone, ipad), account_revoke_other_sessions: () => ({ revoked: 2 }) }))
    await settle(m)
    await press(m, 'Sign out all other sessions')
    await press(m, 'Sign out others')
    await settle(m)
    expect(focused).toEqual([securityModel.SESSIONS_HEADING_ID])
  })

  test('a revoke that failed moves focus nowhere: the row and its button are still there for the sheet to give focus back to', async () => {
    const m = openSecurity(backendOf({ account_revoke_session: () => { throw new Error('server said no') } }))
    await settle(m)
    await press(m, 'Revoke iPhone')
    await press(m, 'Sign out')
    await settle(m)
    expect(focused).toEqual([])
  })

  test('a revoke that fails is ONE toast, the session stays, and no inline error appears', async () => {
    const m = openSecurity(backendOf({ account_revoke_session: () => { throw new Error('server said no') } }))
    await settle(m)
    await press(m, 'Revoke iPhone')
    await press(m, 'Sign out')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t sign out that session' })
    expect(visibleErrorSurfaces(m).filter((s) => s.startsWith('inline'))).toEqual([])
    expect(visibleText(m)).toContain('iPhone')
    expect(dialogs(m)).toHaveLength(0)
  })

  test('Sign out all other sessions asks first, then ends them and counts them in the toast', async () => {
    const m = openSecurity(backendOf({ account_session_list: () => sessionsOf(mac, phone, ipad), account_revoke_other_sessions: () => ({ revoked: 2 }) }))
    await settle(m)
    await press(m, 'Sign out all other sessions')
    expect(called(m, 'account_revoke_other_sessions')).toHaveLength(0)
    expect(visibleText(m)).toContain('Sign out everywhere else?')
    expect(visibleText(m)).toContain('Every other session ends. Those devices will need to sign in again.')
    await press(m, 'Sign out others')
    expect(called(m, 'account_revoke_other_sessions')).toHaveLength(1)
    expect(m.toasts).toEqual([expect.objectContaining({ variant: 'success', title: 'Signed out 2 other sessions' })])
  })

  test('one other session is said in the singular; a failure is one toast', async () => {
    const one = openSecurity(backendOf({ account_revoke_other_sessions: () => ({ revoked: 1 }) }))
    await settle(one)
    await press(one, 'Sign out all other sessions')
    await press(one, 'Sign out others')
    expect(one.toasts[0].title).toBe('Signed out 1 other session')

    const failed = openSecurity(backendOf({ account_revoke_other_sessions: () => { throw new Error('nope') } }))
    await settle(failed)
    await press(failed, 'Sign out all other sessions')
    await press(failed, 'Sign out others')
    expect(failed.toasts).toHaveLength(1)
    expect(failed.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t sign out other sessions' })
  })

  test('a destructive confirm is the red one, as Sign out of this Mac is', async () => {
    const m = openSecurity()
    await settle(m)
    await press(m, 'Revoke iPhone')
    const confirm = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === 'Sign out')
    expect(confirm).toHaveLength(1)
    expect(confirm[0].props.className).toContain('ms-btn--danger')
  })
})

describe('Security: a group that did not load', () => {
  const groups: Array<{ command: string; title: string; surface: string; others: string[] }> = [
    { command: 'account_session_list', title: 'Couldn’t load your active sessions', surface: 'security-sessions', others: ['account_devices', 'account_activity_feed', 'account_profile_fresh'] },
    { command: 'account_devices', title: 'Couldn’t load your devices', surface: 'security-devices', others: ['account_session_list', 'account_activity_feed', 'account_profile_fresh'] },
    { command: 'account_activity_feed', title: 'Couldn’t load recent activity', surface: 'security-activity', others: ['account_session_list', 'account_devices', 'account_profile_fresh'] },
    { command: 'account_profile_fresh', title: 'Couldn’t check your authenticator app', surface: 'security-profile', others: ['account_session_list', 'account_devices', 'account_activity_feed'] },
  ]
  for (const group of groups) {
    test(`${group.command} failing is ONE inline alert with Try again; the other groups are untouched and Try again reads only this one`, async () => {
      let fail = true
      const m = openSecurity(backendOf({ [group.command]: () => { if (fail) throw new Error('offline'); return group.command === 'account_session_list' ? sessionsOf(mac) : group.command === 'account_devices' ? { devices: [device()] } : group.command === 'account_profile_fresh' ? profile(true) : feed([trashEvent]) } }))
      await settle(m)
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${group.title}Try again`])
      expect(find(m, (el) => el.props['data-error-surface'] === group.surface)).toHaveLength(1)
      expect(m.toasts).toEqual([])
      expect(visibleText(m)).not.toContain('offline')
      fail = false
      await press(m, 'Try again')
      expect(called(m, group.command)).toHaveLength(2)
      for (const other of group.others) expect(called(m, other)).toHaveLength(1)
      expect(visibleErrorSurfaces(m)).toEqual([])
    })
  }

  test('an account whose profile carries no totp_enabled is "unknown", not "off"', async () => {
    const m = openSecurity(backendOf({ account_profile_fresh: () => profile(null) }))
    await settle(m)
    expect(visibleText(m)).not.toContain('Authenticator app (TOTP) ·')
    expect(buttons(m)[0]).toBe('Manage on beebeeb.io')
  })


  // PR #140 delta review, P2-B: the row is read from the server every time, and never from what the app cached at sign-in.
  test('a profile that cannot be read is no On and no Off: the row says nothing it does not know, and the alert says so', async () => {
    const m = openSecurity(backendOf({ account_profile_fresh: () => { throw new Error('offline') } }))
    await settle(m)
    expect(visibleText(m)).toContain('Couldn’t check your authenticator app')
    expect(visibleText(m)).not.toContain('Authenticator app (TOTP) ·')
    expect(visibleText(m)).not.toMatch(/ · On| · Off/)
    expect(visibleText(m)).not.toContain('offline')
    expect(m.toasts).toEqual([])
    expect(visibleText(m)).toContain('Active sessions')
  })
})

describe('Security: the authenticator row is read fresh (review P2-B) and not more often than a minute (review P3-4)', () => {
  /** The page's clock, which the re-check interval reads (`Date.now()`); the real one is back after every test. */
  const realNow = Date.now
  let at = 1_800_000_000_000
  const advance = (ms: number) => { at += ms }
  beforeEach(() => { at = 1_800_000_000_000; Date.now = () => at })
  afterEach(() => { Date.now = realNow })
  const MINUTE = 60_000

  /** The `focus` listeners the page registered on the window, so a test can focus it. */
  function watchFocus(): { fire: () => void; live: () => number } {
    const listeners = new Set<() => void>()
    const w = (globalThis as any).window
    w.addEventListener = (name: string, fn: () => void) => { if (name === 'focus') listeners.add(fn) }
    w.removeEventListener = (name: string, fn: () => void) => { if (name === 'focus') listeners.delete(fn) }
    return { fire: () => listeners.forEach((fn) => fn()), live: () => listeners.size }
  }

  test('it asks the server and never the session\'s cache: account_profile_fresh, and account_profile is not read', async () => {
    const m = openSecurity()
    await settle(m)
    expect(called(m, 'account_profile_fresh')).toHaveLength(1)
    expect(called(m, 'account_profile')).toHaveLength(0)
  })

  test('until the answer arrives it says "Checking…" and claims neither On nor Off', async () => {
    const m = openSecurity(backendOf({ account_profile_fresh: () => new Promise(() => {}) }))
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP)')
    expect(visibleText(m)).toContain('Checking…')
    expect(visibleText(m)).not.toMatch(/Authenticator app \(TOTP\) · /)
    expect(visibleText(m)).not.toContain('A second factor is required')
    expect(visibleText(m)).not.toContain('Add a second factor')
  })

  test('turned off on the web, it says Off the next time the window is looked at: the answer replaces what it said', async () => {
    let on = true
    const m = openSecurity(backendOf({ account_profile_fresh: () => profile(on) }))
    const focus = watchFocus()
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
    on = false
    advance(MINUTE + 1000)
    focus.fire()
    await settle(m)
    expect(called(m, 'account_profile_fresh')).toHaveLength(2)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
    expect(visibleText(m)).not.toContain('Authenticator app (TOTP) · On')
  })

  // P3-4: a failed quiet re-check changes nothing. The answer on screen was seen; an alert would pop up under a person who
  // asked for nothing, and one failed request on a flaky connection would replace a correct row.
  test('a re-check made by looking at the window that fails keeps the last answer and raises no alert', async () => {
    let fail = false
    const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(true) } }))
    const focus = watchFocus()
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
    fail = true
    advance(MINUTE + 1000)
    focus.fire()
    await settle(m)
    expect(called(m, 'account_profile_fresh')).toHaveLength(2)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
    expect(visibleText(m)).not.toContain('Couldn’t check your authenticator app')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('looking at the window again within a minute does not ask the server again', async () => {
    const m = openSecurity()
    const focus = watchFocus()
    await settle(m)
    for (let look = 0; look < 5; look++) {
      advance(10_000)
      focus.fire()
      await settle(m)
    }
    expect(called(m, 'account_profile_fresh')).toHaveLength(1)
    advance(MINUTE)
    focus.fire()
    await settle(m)
    expect(called(m, 'account_profile_fresh')).toHaveLength(2)
  })

  test('a first look that failed is the alert, and a later look at the window that works replaces it', async () => {
    let fail = true
    const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(false) } }))
    const focus = watchFocus()
    await settle(m)
    expect(visibleText(m)).toContain('Couldn’t check your authenticator app')
    fail = false
    advance(MINUTE + 1000)
    focus.fire()
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('a first look that failed stays the alert when the next quiet look fails too', async () => {
    const m = openSecurity(backendOf({ account_profile_fresh: () => { throw new Error('offline') } }))
    const focus = watchFocus()
    await settle(m)
    advance(MINUTE + 1000)
    focus.fire()
    await settle(m)
    expect(visibleText(m)).toContain('Couldn’t check your authenticator app')
    expect(visibleText(m)).not.toMatch(/Authenticator app \(TOTP\) · /)
  })

  // Lead ruling on #147: the age of a stale answer is said, in the row's secondary text.
  describe('an answer older than five minutes says so', () => {
    /** The page's own minute tick (it re-renders so a shown age does not stand still), so a test can let a minute pass. */
    function watchTicks(): { tick: () => void; live: () => number } {
      const fns = new Set<() => void>()
      const w = (globalThis as any).window
      w.setInterval = (fn: () => void) => { fns.add(fn); return fns.size }
      w.clearInterval = () => { fns.clear() }
      return { tick: () => fns.forEach((fn) => fn()), live: () => fns.size }
    }

    test('under five minutes the row is as it was: no age', async () => {
      const m = openSecurity()
      watchFocus()
      await settle(m)
      advance(5 * MINUTE - 1000)
      m.render()
      expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
      expect(visibleText(m)).not.toContain('Last checked')
    })

    test('a quiet re-check that fails after six minutes keeps the answer and adds "Last checked 6 minutes ago."', async () => {
      let fail = false
      const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(true) } }))
      const focus = watchFocus()
      await settle(m)
      fail = true
      advance(6 * MINUTE)
      focus.fire()
      await settle(m)
      expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
      expect(visibleText(m)).toContain('A second factor is required when you sign in. Manage or reset it on beebeeb.io. Last checked 6 minutes ago.')
      expect(visibleErrorSurfaces(m)).toEqual([])
    })

    test('from exactly five minutes, and not a second before', async () => {
      const m = openSecurity()
      const ticks = watchTicks()
      watchFocus()
      await settle(m)
      advance(5 * MINUTE - 1000)
      ticks.tick()
      await settle(m)
      expect(visibleText(m)).not.toContain('Last checked')
      advance(1000)
      ticks.tick()
      await settle(m)
      expect(visibleText(m)).toContain('Last checked 5 minutes ago.')
    })

    test('the age keeps growing while nobody looks: the page re-renders every minute', async () => {
      const m = openSecurity(backendOf({ account_profile_fresh: () => profile(false) }))
      const ticks = watchTicks()
      watchFocus()
      await settle(m)
      expect(ticks.live()).toBe(1)
      advance(10 * MINUTE)
      ticks.tick()
      await settle(m)
      expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
      expect(visibleText(m)).toContain('Set it up on beebeeb.io. Last checked 10 minutes ago.')
      advance(2 * 60 * MINUTE)
      ticks.tick()
      await settle(m)
      expect(visibleText(m)).toContain('Last checked 2 hours ago.')
    })

    test('an answer that works again is fresh again: the age is gone', async () => {
      let fail = false
      const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(true) } }))
      const focus = watchFocus()
      await settle(m)
      fail = true
      advance(6 * MINUTE)
      focus.fire()
      await settle(m)
      expect(visibleText(m)).toContain('Last checked 6 minutes ago.')
      fail = false
      advance(MINUTE + 1000)
      focus.fire()
      await settle(m)
      expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
      expect(visibleText(m)).not.toContain('Last checked')
    })

    test('only a state that is claimed carries an age: not "Checking…", not an answer that says nothing, not the alert', async () => {
      const unknown = openSecurity(backendOf({ account_profile_fresh: () => profile(null) }))
      const ticks = watchTicks()
      await settle(unknown)
      advance(20 * MINUTE)
      ticks.tick()
      await settle(unknown)
      expect(visibleText(unknown)).not.toContain('Last checked')
      const failed = openSecurity(backendOf({ account_profile_fresh: () => { throw new Error('offline') } }))
      const failedTicks = watchTicks()
      await settle(failed)
      advance(20 * MINUTE)
      failedTicks.tick()
      await settle(failed)
      expect(visibleText(failed)).not.toContain('Last checked')
    })

    test('a locked vault runs no tick', async () => {
      const m = openSecurity(backendOf(), { unlocked: false })
      const ticks = watchTicks()
      await settle(m)
      expect(ticks.live()).toBe(0)
    })

    test('leaving the page stops the tick', async () => {
      const m = openSecurity()
      const ticks = watchTicks()
      await settle(m)
      expect(ticks.live()).toBe(1)
      m.unmount()
      expect(ticks.live()).toBe(0)
    })
  })

  test('Try again is the person asking: it reads at once, inside the minute, and its failure is the alert', async () => {
    let fail = true
    const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(true) } }))
    await settle(m)
    advance(5_000)
    await press(m, 'Try again')
    expect(called(m, 'account_profile_fresh')).toHaveLength(2)
    expect(visibleText(m)).toContain('Couldn’t check your authenticator app')
    fail = false
    advance(5_000)
    await press(m, 'Try again')
    expect(called(m, 'account_profile_fresh')).toHaveLength(3)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · On')
  })

  test('a slow answer from an earlier look cannot overwrite a later one', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    let n = 0
    const m = openSecurity(backendOf({ account_profile_fresh: async () => { n += 1; if (n === 1) { await gate; return profile(true) } return profile(false) } }))
    const focus = watchFocus()
    await m.flush()
    advance(MINUTE + 1000)
    focus.fire()
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
    release()
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
    expect(visibleText(m)).not.toContain('Authenticator app (TOTP) · On')
  })

  test('Try again reads again, and only the authenticator', async () => {
    let fail = true
    const m = openSecurity(backendOf({ account_profile_fresh: () => { if (fail) throw new Error('offline'); return profile(false) } }))
    await settle(m)
    fail = false
    await press(m, 'Try again')
    expect(visibleText(m)).toContain('Authenticator app (TOTP) · Off')
    expect(called(m, 'account_session_list')).toHaveLength(1)
  })

  test('a locked vault looks at nothing and listens for nothing', async () => {
    const m = openSecurity(backendOf(), { unlocked: false })
    const focus = watchFocus()
    await settle(m)
    expect(focus.live()).toBe(0)
    expect(m.calls).toEqual([])
  })

  test('leaving the page stops listening', async () => {
    const m = openSecurity()
    const focus = watchFocus()
    await settle(m)
    expect(focus.live()).toBe(1)
    m.unmount()
    expect(focus.live()).toBe(0)
  })
})
