// P1c, slice 02: the account panes start from the app's own empty defaults, never the board's example account, and the
// four production modules no longer import the board's fixtures. Every value in the running app comes from a command.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { initialAccountState } from '../src/v2/sections/settings/account/data'

const FILES = ['data.ts', 'ui.tsx', 'Factors.tsx', 'SecurityPane.tsx']

describe('the account panes start from the app’s own empty state (P1c, slice 02)', () => {
  test('initialAccountState() is empty, with one product row: this app’s Drive', () => {
    const s = initialAccountState()
    expect(s.name).toBe('')
    expect(s.email).toBe('')
    expect(s.emailVerified).toBe(false)
    expect(s.passwordChangedAt).toBeNull()
    expect(s.totp.on).toBe(false)
    expect(s.totp.codesTotal).toBe(8)
    expect(s.devices.length).toBe(0)
    expect(s.passkeys.length).toBe(0)
    expect(s.factors.length).toBe(0)
    expect(s.activity.length).toBe(0)
    expect(s.activityLogging).toBe(true)
    expect(s.products).toEqual([{ name: 'Drive', state: 'This app' }])
  })

  test('none of the four account modules reaches the board’s fixtures or a browser answer', () => {
    const hits = FILES.filter((f) => readFileSync(new URL(`../src/v2/sections/settings/account/${f}`, import.meta.url), 'utf8').includes('source/fixtures'))
    expect(hits).toEqual([])
    // Review round 1: a `null` third argument makes run() answer ok in a plain browser, so the pane claims the
    // password changed or verified when no command ran. Every call in these files must go through the command.
    const security = readFileSync(new URL('../src/v2/sections/settings/account/SecurityPane.tsx', import.meta.url), 'utf8')
    expect(security).not.toContain("'account_change_password', { current: c, newPassword: next }, null")
    expect(security).not.toContain("'account_delete_check_password', { password: pw }, null")
  })
})
