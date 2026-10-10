/**
 * The onboarding window closes itself through JS in two places (a same-account re-sign-in, and
 * Cancel on the switch warning in reauth mode), and the ReadyStep closes it too. Tauri's
 * `core:window:default` does not include `allow-close`, so without an explicit grant the ACL rejects
 * `getCurrentWindow().close()` and the window stays open on the form the person just completed.
 *
 * Least privilege (lead ruling FT-I1): exactly one capability grants `core:window:allow-close`, it
 * names the `onboarding` window and nothing else, and it grants nothing else. No other capability
 * may grant it (or the window plugin wholesale).
 *
 * What this proves: what the capability files say. What it does NOT prove: that Tauri loads them
 * (tauri_build reads every file in src-tauri/capabilities/ when tauri.conf.json lists none, which
 * the last test pins) or that the window closes on a Mac (device checks D5 and D5b).
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'

const dir = new URL('../src-tauri/capabilities/', import.meta.url)

interface Capability {
  identifier: string
  windows?: string[]
  permissions: Array<string | { identifier: string }>
}

function capabilities(): Array<{ file: string; cap: Capability }> {
  return readdirSync(dir)
    .filter((file) => file.endsWith('.json'))
    .sort()
    .map((file) => ({ file, cap: JSON.parse(readFileSync(new URL(file, dir), 'utf8')) as Capability }))
}

const permissionIds = (cap: Capability) => cap.permissions.map((p) => (typeof p === 'string' ? p : p.identifier))

describe('the onboarding window may close itself, and only it', () => {
  test('exactly one capability grants core:window:allow-close', () => {
    const granting = capabilities().filter(({ cap }) => permissionIds(cap).includes('core:window:allow-close'))
    expect(granting.map(({ file }) => file)).toEqual(['onboarding-close.json'])
  })

  test('that capability names the onboarding window alone and grants nothing else', () => {
    const found = capabilities().find(({ file }) => file === 'onboarding-close.json')
    expect(found).toBeDefined()
    const cap = found!.cap
    expect(cap.identifier).toBe('onboarding-close')
    expect(cap.windows).toEqual(['onboarding'])
    expect(permissionIds(cap)).toEqual(['core:window:allow-close'])
  })

  test('no capability grants the window plugin wholesale or a broader close', () => {
    const broad = ['core:window:allow-destroy', 'core:window:allow-*', 'core:window:*']
    for (const { file, cap } of capabilities()) {
      const ids = permissionIds(cap)
      expect({ file, broad: ids.filter((id) => broad.includes(id)) }).toEqual({ file, broad: [] })
    }
  })

  test('tauri.conf.json lists no capabilities of its own, so tauri_build loads every file in the folder', () => {
    const conf = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'))
    expect(conf.app?.security?.capabilities).toBeUndefined()
  })
})
