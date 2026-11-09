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

  test('none of the four account modules reaches the board’s fixtures', () => {
    const hits = FILES.filter((f) => readFileSync(new URL(`../src/v2/sections/settings/account/${f}`, import.meta.url), 'utf8').includes('source/fixtures'))
    expect(hits).toEqual([])
  })
})
