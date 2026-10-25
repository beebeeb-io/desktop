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
import { afterEach, describe, expect, test } from 'bun:test'
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
  while (mounted.length) mounted.pop()!.close()
})

const hoursAgo = (n: number) => new Date(Date.now() - n * 3_600_000).toISOString()
const today = (h: number, m: number) => { const d = new Date(); d.setHours(h, m, 0, 0); return d }
const hhmm = (d: Date) => `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`

const score = (on: boolean) => ({ score: 4, max: 5, label: 'Good', factors: [{ key: 'email_verified', satisfied: true }, { key: 'two_factor_enabled', satisfied: on }] })
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
    account_security_score: () => score(true),
    account_session_list: () => sessionsOf(mac, phone),
    account_devices: () => ({ devices: [device()] }),
    account_activity_feed: () => feed([trashEvent]),
    account_revoke_session: () => undefined,
    account_revoke_other_sessions: () => ({ revoked: 1 }),
    'plugin:opener|open_url': () => undefined,
    ...over,
  }
}

function openSecurity(backend: Record<string, (args: any) => unknown> = backendOf(), props: object = { unlocked: true }) {
  const m = mount('MacSecurity.tsx', 'SecurityTab', {
    backend: backend as any,
    expand: true,
    props,
    bindings: { ...desktopApi, ...securityModel, ...parts, ConfirmSheet },
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
  test('reads the score, the sessions, the devices and the last ten events once each, and nothing else', async () => {
    const m = openSecurity()
    await settle(m)
    expect(m.calls.map((c) => c.name).sort()).toEqual(['account_activity_feed', 'account_devices', 'account_security_score', 'account_session_list'])
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

    const off = openSecurity(backendOf({ account_security_score: () => score(false) }))
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
    { command: 'account_session_list', title: 'Couldn’t load your active sessions', surface: 'security-sessions', others: ['account_devices', 'account_activity_feed', 'account_security_score'] },
    { command: 'account_devices', title: 'Couldn’t load your devices', surface: 'security-devices', others: ['account_session_list', 'account_activity_feed', 'account_security_score'] },
    { command: 'account_activity_feed', title: 'Couldn’t load recent activity', surface: 'security-activity', others: ['account_session_list', 'account_devices', 'account_security_score'] },
  ]
  for (const group of groups) {
    test(`${group.command} failing is ONE inline alert with Try again; the other groups are untouched and Try again reads only this one`, async () => {
      let fail = true
      const m = openSecurity(backendOf({ [group.command]: () => { if (fail) throw new Error('offline'); return group.command === 'account_session_list' ? sessionsOf(mac) : group.command === 'account_devices' ? { devices: [device()] } : feed([trashEvent]) } }))
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

  test('a score that cannot be read leaves the authenticator row honest ("Manage…") and raises no error', async () => {
    const m = openSecurity(backendOf({ account_security_score: () => { throw new Error('offline') } }))
    await settle(m)
    expect(visibleText(m)).toContain('Authenticator app (TOTP)')
    expect(visibleText(m)).not.toContain('Authenticator app (TOTP) ·')
    expect(visibleText(m)).toContain('Manage two-factor authentication on beebeeb.io.')
    expect(buttons(m)[0]).toBe('Manage on beebeeb.io')
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(visibleText(m)).toContain('Active sessions')
  })
})
