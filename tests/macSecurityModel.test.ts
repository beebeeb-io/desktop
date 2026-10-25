/**
 * The words and the arithmetic behind Settings › Security (task 1683 slice 5; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8 and §10.2 #29; hi-fi section 4, "Security (new)"): what the
 * authenticator row says in each of its three states, how a session, a device and an event read, how long ago is said.
 * Pure functions, so every sentence is pinned here without a window. The copy is the Windows shell's (`SecurityView`,
 * `DevicesView`, `ActivityView`), kept; the Devices subtitle is spec #29's replacement, because no revoke control exists.
 */
import { describe, expect, test } from 'bun:test'
import {
  ACTIVITY_LIMIT,
  activityRows,
  agoLabel,
  DEVICES_FOOTNOTE,
  deviceRow,
  SECURITY_SETTINGS_URL,
  sessionRow,
  TOTP_LABEL,
  totpAction,
  totpHint,
  totpLabel,
  totpRowState,
  totpState,
} from '../src/macSecurityModel'

/** A local wall-clock moment, so the day and clock words do not depend on the machine's zone. */
const at = (y: number, mo: number, d: number, h = 0, mi = 0, s = 0) => new Date(y, mo - 1, d, h, mi, s)
const now = at(2026, 10, 25, 14, 30).getTime()
const iso = (date: Date) => date.toISOString()

describe('the authenticator row', () => {
  test('its state is the account\'s own totp_enabled: on, off, or unknown when the account does not say (review P2-5)', () => {
    expect(totpState({ totp_enabled: true })).toBe('on')
    expect(totpState({ totp_enabled: false })).toBe('off')
    expect(totpState({ totp_enabled: null })).toBe('unknown')
    expect(totpState({})).toBe('unknown')
    expect(totpState(null)).toBe('unknown')
    expect(totpState(undefined)).toBe('unknown')
  })

  test('a security-score factor never decides it: "two_factor_enabled" is an authenticator OR a passkey, and ignores whether the authenticator is on', () => {
    // The score is not an input any more: a payload that carries factors but no totp_enabled is "unknown", not "on".
    expect(totpState({ factors: [{ key: 'two_factor_enabled', satisfied: true }] } as never)).toBe('unknown')
    expect(totpState({ totp_enabled: false, factors: [{ key: 'two_factor_enabled', satisfied: true }] } as never)).toBe('off')
  })

  test('the words, per state: copy kept from the Windows shell, with "Manage" or "Set up" on beebeeb.io', () => {
    expect(TOTP_LABEL).toBe('Authenticator app (TOTP)')
    expect(totpLabel('on')).toBe('Authenticator app (TOTP) · On')
    expect(totpLabel('off')).toBe('Authenticator app (TOTP) · Off')
    expect(totpLabel('unknown')).toBe('Authenticator app (TOTP)')
    expect(totpHint('on')).toBe('A second factor is required when you sign in. Manage or reset it on beebeeb.io.')
    expect(totpHint('off')).toBe('Add a second factor so a stolen password alone can’t open your vault. Set it up on beebeeb.io.')
    expect(totpHint('unknown')).toBe('Manage two-factor authentication on beebeeb.io.')
    expect(totpAction('on')).toBe('Manage on beebeeb.io')
    expect(totpAction('off')).toBe('Set up on beebeeb.io')
    expect(totpAction('unknown')).toBe('Manage on beebeeb.io')
    expect(SECURITY_SETTINGS_URL).toBe('https://app.beebeeb.io/settings/security')
  })

  // PR #140 delta review, P2-B: the row is what the server says now. Until it answers it is "Checking…", and a failed look is
  // not a row at all (an alert with Try again says so): no state shows an older answer as current.
  test('the row follows the look: checking while it runs, nothing to show when it failed, the account\'s own answer otherwise', () => {
    expect(totpRowState({ status: 'loading' })).toBe('checking')
    expect(totpRowState({ status: 'failed' })).toBe('failed')
    expect(totpRowState({ status: 'ready', value: { totp_enabled: true } })).toBe('on')
    expect(totpRowState({ status: 'ready', value: { totp_enabled: false } })).toBe('off')
    expect(totpRowState({ status: 'ready', value: { totp_enabled: null } })).toBe('unknown')
  })

  test('"Checking…" says it is checking and claims nothing; the link is the same neutral one', () => {
    expect(totpLabel('checking')).toBe('Authenticator app (TOTP)')
    expect(totpHint('checking')).toBe('Checking…')
    expect(totpAction('checking')).toBe('Manage on beebeeb.io')
  })

  test('there is no in-app toggle to promise: no state asks for a code or a switch', () => {
    for (const state of ['on', 'off', 'unknown', 'checking'] as const) {
      expect(`${totpLabel(state)} ${totpHint(state)} ${totpAction(state)}`).not.toMatch(/enable|disable|turn (on|off)|scan|enter (a|the) code/i)
    }
  })
})

describe('how long ago is said', () => {
  test('just now under 45 seconds, and for a clock that is ahead', () => {
    expect(agoLabel(iso(at(2026, 10, 25, 14, 29, 40)), now)).toBe('just now')
    expect(agoLabel(iso(at(2026, 10, 25, 14, 31)), now)).toBe('just now')
  })

  test('minutes, hours and days in whole words, singular where it is one', () => {
    expect(agoLabel(iso(at(2026, 10, 25, 14, 29)), now)).toBe('1 minute ago')
    expect(agoLabel(iso(at(2026, 10, 25, 14, 25)), now)).toBe('5 minutes ago')
    expect(agoLabel(iso(at(2026, 10, 25, 13, 30)), now)).toBe('1 hour ago')
    expect(agoLabel(iso(at(2026, 10, 25, 12, 30)), now)).toBe('2 hours ago')
    expect(agoLabel(iso(at(2026, 10, 24, 14, 30)), now)).toBe('1 day ago')
    expect(agoLabel(iso(at(2026, 10, 22, 14, 30)), now)).toBe('3 days ago')
  })

  test('after a month a date: no year in the current year, the year otherwise; English month names on any Mac', () => {
    expect(agoLabel(iso(at(2026, 9, 3, 10)), now)).toBe('3 Sep')
    expect(agoLabel(iso(at(2025, 9, 3, 10)), now)).toBe('3 Sep 2025')
    expect(agoLabel(iso(at(2026, 5, 1, 10)), now)).toBe('1 May')
  })

  test('a missing or unreadable time is null, never "NaN" or "Invalid Date"', () => {
    expect(agoLabel(null, now)).toBeNull()
    expect(agoLabel(undefined, now)).toBeNull()
    expect(agoLabel('', now)).toBeNull()
    expect(agoLabel('not a date', now)).toBeNull()
  })
})

describe('an active session', () => {
  const session = (over: object = {}) => ({ id: 's1', device_name: 'iPhone', device_kind: 'ios', country_code: 'NL', last_active_at: iso(at(2026, 10, 25, 12, 30)), created_at: iso(at(2026, 9, 1, 10)), is_current: false, ...over })

  test('this session is named so and is active now; another one says how long ago', () => {
    expect(sessionRow(session({ id: 'c', device_name: 'MacBook Pro', is_current: true }), now)).toEqual({ id: 'c', title: 'MacBook Pro · This session', hint: 'Active now', current: true })
    expect(sessionRow(session(), now)).toEqual({ id: 's1', title: 'iPhone', hint: 'Active 2 hours ago', current: false })
  })

  test('a session that was never active falls back to when it was created; an unnamed one is "Unknown device"', () => {
    expect(sessionRow(session({ last_active_at: null, created_at: iso(at(2026, 10, 24, 14, 30)) }), now).hint).toBe('Active 1 day ago')
    expect(sessionRow(session({ device_name: '' }), now).title).toBe('Unknown device')
  })

  test('a time that cannot be read says so instead of inventing one', () => {
    expect(sessionRow(session({ last_active_at: null, created_at: 'garbage' }), now).hint).toBe('Last active time unknown')
  })
})

describe('a device', () => {
  const device = (over: object = {}) => ({ id: 'd1', hostname: 'MacBook Pro', platform: 'macos', bb_version: '0.8.13', last_seen: iso(at(2026, 10, 25, 14, 30)), created_at: iso(at(2026, 9, 3, 9)), session_count: 0, ...over })

  test('the hi-fi\'s line: name, platform and version, then when it was last seen and registered', () => {
    expect(deviceRow(device(), now)).toEqual({ id: 'd1', title: 'MacBook Pro · macOS · v0.8.13', hint: 'last seen just now · registered 3 Sep' })
  })

  test('platforms read the way people write them; an unknown one is not guessed', () => {
    expect(deviceRow(device({ platform: 'darwin' }), now).title).toContain('· macOS ·')
    expect(deviceRow(device({ platform: 'windows' }), now).title).toContain('· Windows ·')
    expect(deviceRow(device({ platform: 'ios' }), now).title).toContain('· iOS ·')
    expect(deviceRow(device({ platform: 'android' }), now).title).toContain('· Android ·')
    expect(deviceRow(device({ platform: '' }), now).title).toBe('MacBook Pro · Unknown platform · v0.8.13')
    expect(deviceRow(device({ bb_version: null }), now).title).toBe('MacBook Pro · macOS')
  })

  test('the sync sessions the footnote promises are counted on the row, and only when there are some', () => {
    expect(deviceRow(device({ session_count: 1 }), now).hint).toBe('last seen just now · registered 3 Sep · 1 sync session')
    expect(deviceRow(device({ session_count: 3 }), now).hint).toBe('last seen just now · registered 3 Sep · 3 sync sessions')
    expect(deviceRow(device({ session_count: 0 }), now).hint).not.toContain('sync session')
  })

  test('spec #29: the subtitle claims no revoke, because no control to revoke a device exists', () => {
    expect(DEVICES_FOOTNOTE).toBe('Every device holding a copy of your vault key, and the sync sessions running on each.')
    expect(DEVICES_FOOTNOTE).not.toMatch(/revoke|re-encrypt/i)
  })
})

describe('recent account activity', () => {
  const event = (type: string, createdAt: Date, over: object = {}) => ({ id: `${type}@${createdAt.getTime()}`, type, subject: null, details: null, where: null, created_at: iso(createdAt), ...over })

  test('shows the last ten events, newest first, whatever order they arrive in', () => {
    const events = Array.from({ length: 14 }, (_, i) => event('file.downloaded', at(2026, 10, 25, 8, i)))
    const rows = activityRows([...events].reverse(), now)
    expect(ACTIVITY_LIMIT).toBe(10)
    expect(rows).toHaveLength(10)
    expect(rows.map((row) => row.id)).toEqual(events.slice(4).reverse().map((e) => e.id))
  })

  test('known events read as a person would say them; a bulk trash counts its items', () => {
    const when = at(2026, 10, 25, 9, 12)
    expect(activityRows([event('file.trash', when)], now)[0].title).toBe('Moved an item to trash')
    expect(activityRows([event('file.downloaded', when)], now)[0].title).toBe('Downloaded an item')
    expect(activityRows([event('auth.logout', when)], now)[0].title).toBe('Signed out')
    expect(activityRows([event('user.logout', when)], now)[0].title).toBe('Signed out')
    expect(activityRows([event('file.trash.bulk', when, { subject: 'a1111111-1111-4111-8111-111111111111, b2222222-2222-4222-8222-222222222222, c3333333-3333-4333-8333-333333333333' })], now)[0].title).toBe('Moved 3 items to trash')
    expect(activityRows([event('file.trash.bulk', when, { subject: 'a1111111-1111-4111-8111-111111111111' })], now)[0].title).toBe('Moved an item to trash')
  })

  test('a subject that is a name is the title; a raw id never reaches the screen', () => {
    const when = at(2026, 10, 25, 9, 12)
    expect(activityRows([event('file.upload', when, { subject: 'Q3 ledger.xlsx' })], now)[0].title).toBe('Q3 ledger.xlsx')
    const id = activityRows([event('auth.login.success', when, { subject: 'a1111111-1111-4111-8111-111111111111' })], now)[0]
    expect(id.title).toBe('Auth login success')
    expect(JSON.stringify(id)).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}/)
    expect(activityRows([event('', when)], now)[0].title).toBe('Activity')
  })

  test('when: today and yesterday with the clock, then a date', () => {
    expect(activityRows([event('file.trash', at(2026, 10, 25, 9, 12))], now)[0].when).toBe('Today 09:12')
    expect(activityRows([event('file.trash', at(2026, 10, 24, 18, 3))], now)[0].when).toBe('Yesterday 18:03')
    expect(activityRows([event('file.trash', at(2026, 9, 3, 7, 5))], now)[0].when).toBe('3 Sep 07:05')
    expect(activityRows([event('file.trash', at(2025, 9, 3, 7, 5))], now)[0].when).toBe('3 Sep 2025 07:05')
  })

  test('an event whose time cannot be read is kept, last, and says nothing about when', () => {
    const rows = activityRows([event('file.trash', at(2026, 10, 25, 9, 12)), { ...event('file.downloaded', at(2026, 10, 25, 9, 0)), created_at: 'garbage' }], now)
    expect(rows.map((row) => row.title)).toEqual(['Moved an item to trash', 'Downloaded an item'])
    expect(rows[1].when).toBeNull()
  })
})
