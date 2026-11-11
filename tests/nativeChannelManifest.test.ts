// The v2 channel manifest a native Windows release publishes (scripts/native-channel-manifest.mjs). The fixture it
// must reproduce byte for byte is the one the updater crate parses in its own tests
// (crates/beebeeb-updater/src/manifest.rs), so the script and the crate cannot drift apart unnoticed.
import { describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { buildManifest, parseVersion, signedFileName } from '../scripts/native-channel-manifest.mjs'

const FIXTURE = path.join(import.meta.dir, '..', 'crates', 'beebeeb-updater', 'tests', 'fixtures', 'native-manifest-shape.json')
const NAME = 'Beebeeb_0.9.0_x64-setup.exe'
/** Shaped like a Tauri `.sig` (base64 of four lines); not a real signature. */
const sigFor = (file: string) =>
  Buffer.from(`untrusted comment: signature from tauri secret key\nAAAA\ntrusted comment: timestamp:1800000000\tfile:${file}\nBBBB\n`).toString('base64')
const honest = {
  version: '0.9.0',
  channel: 'beta',
  installerName: NAME,
  installerSize: 9_000_001,
  signature: sigFor(NAME),
  notes: '# Beebeeb Desktop 0.9.0 - native\n',
  date: '2026-11-20T10:00:00Z',
}

describe('native channel manifest', () => {
  test('writes the shape the updater crate reads', () => {
    const written = `${JSON.stringify(buildManifest(honest), null, 2)}\n`
    expect(written).toBe(fs.readFileSync(FIXTURE, 'utf8'))
  })

  test('points both Windows targets at our own host and names product, channel and size', () => {
    const manifest = buildManifest(honest)
    expect(Object.keys(manifest.platforms).sort()).toEqual(['windows-x86_64', 'windows-x86_64-nsis'])
    for (const platform of Object.values(manifest.platforms)) {
      expect(platform.url).toBe(`https://releases.beebeeb.io/desktop/artifacts/0.9.0/${NAME}`)
      expect(platform.size).toBe(9_000_001)
    }
    expect([manifest.product, manifest.channel, manifest.version]).toEqual(['desktop', 'beta', '0.9.0'])
  })

  test('refuses each bad input by name', () => {
    const refusal = (change: Partial<typeof honest>) => {
      try {
        buildManifest({ ...honest, ...change })
      } catch (error) {
        return (error as Error).message
      }
      return 'accepted'
    }
    expect(refusal({})).toBe('accepted')
    expect(refusal({ version: '0.9.0-beta.1' })).toContain('plain semver')
    expect(refusal({ version: '0.8.25', installerName: 'Beebeeb_0.8.25_x64-setup.exe', signature: sigFor('Beebeeb_0.8.25_x64-setup.exe'), notes: '0.8.25' })).toContain('older than the first native release')
    expect(refusal({ channel: 'nightly' })).toContain('alpha, beta or stable')
    expect(refusal({ installerName: 'Beebeeb Test_0.9.0_x64-setup.exe' })).toContain('must be named')
    expect(refusal({ installerSize: 0 })).toContain('empty')
    expect(refusal({ signature: sigFor('Beebeeb_0.8.22_x64-setup.exe') })).toContain("signs 'Beebeeb_0.8.22_x64-setup.exe'")
    expect(refusal({ signature: 'PGh0bWw+' })).toContain("signs 'null'")
    expect(refusal({ date: 'yesterday' })).toContain('must look like')
    expect(refusal({ notes: '# Beebeeb Desktop 0.8.9' })).toContain('do not name 0.9.0')
  })

  test('reads versions and signed file names strictly', () => {
    expect(parseVersion('0.10.2')).toEqual([0, 10, 2])
    expect(parseVersion('01.0.0')).toBeNull()
    expect(signedFileName(sigFor('a.exe'))).toBe('a.exe')
    expect(signedFileName('')).toBeNull()
  })
})
