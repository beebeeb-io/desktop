/**
 * Task 1747 Part 2: `vault_unlock_route`, the Keychain-presence signal, as the Beebeeb window reads it.
 *
 * Rust answers `keychain` (a vault key is in this Mac's Keychain, in either layout, or the Keychain could not be read: the
 * existing unlock error is then what the person reads, rather than a path that would overwrite a key) or `recovery_phrase`
 * (neither layout holds a key). The TypeScript side must fail the same way: anything it cannot read is `keychain`, and the
 * one value that sends the person to Set up this Mac is the exact string Rust sends.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { parseVaultUnlockRoute, vaultUnlockRoute } from '../src/desktopApi'
import { rustSource } from './fixtures/rustConstants'

describe('parseVaultUnlockRoute: fails closed to the Keychain unlock', () => {
  test('the two values Rust sends', () => {
    expect(parseVaultUnlockRoute('keychain')).toBe('keychain')
    expect(parseVaultUnlockRoute('recovery_phrase')).toBe('recovery_phrase')
  })

  test('anything else is the Keychain unlock, never Set up this Mac', () => {
    for (const raw of [undefined, null, '', 'Keychain', 'RecoveryPhrase', 'recovery-phrase', ' recovery_phrase', 0, true, {}, ['recovery_phrase'], { route: 'recovery_phrase' }]) {
      expect({ raw, route: parseVaultUnlockRoute(raw) }).toEqual({ raw, route: 'keychain' })
    }
  })

  test('the strings are the ones Rust serializes (the enum is snake_case on the wire)', () => {
    const lib = rustSource('lib.rs')
    const start = lib.indexOf('enum VaultUnlockRoute')
    expect(start, 'lib.rs declares VaultUnlockRoute').toBeGreaterThan(-1)
    const header = lib.slice(Math.max(0, start - 200), start)
    expect(header).toContain('rename_all = "snake_case"')
    const body = lib.slice(start, lib.indexOf('}', start))
    expect(body).toContain('Keychain')
    expect(body).toContain('RecoveryPhrase')
  })
})

describe('vaultUnlockRoute: the one command, read as an answer about the active account', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })
  const backend = (invoke: (name: string, args: unknown) => unknown) => {
    const calls: Array<{ name: string; args: unknown }> = []
    ;(globalThis as any).window = {
      __TAURI_INTERNALS__: { invoke: async (name: string, args: unknown) => { calls.push({ name, args }); return invoke(name, args) } },
    }
    return calls
  }

  test('asks vault_unlock_route and nothing else, with no arguments (the active account is Rust\'s)', async () => {
    const calls = backend(() => 'recovery_phrase')
    expect(await vaultUnlockRoute()).toEqual({ ok: true, value: 'recovery_phrase' })
    expect(calls).toEqual([{ name: 'vault_unlock_route', args: {} }])
  })

  test('an unreadable answer is the Keychain unlock', async () => {
    backend(() => ({ route: 'recovery_phrase' }))
    expect(await vaultUnlockRoute()).toEqual({ ok: true, value: 'keychain' })
  })

  test('a build without the command is `unsupported`, so a caller can tell it from a refused answer', async () => {
    backend(() => { throw new Error('unknown command vault_unlock_route') })
    const result = await vaultUnlockRoute()
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.unsupported).toBe(true)
  })
})
