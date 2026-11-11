// The v2 channel manifest a native Windows release publishes (scripts/native-channel-manifest.mjs). The fixture it
// must reproduce byte for byte is the one the updater crate parses in its own tests
// (crates/beebeeb-updater/src/manifest.rs), so the script and the crate cannot drift apart unnoticed.
import { describe, expect, test } from 'bun:test'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

import { buildManifest, parseVersion, signedFileName, verifyMinisign } from '../scripts/native-channel-manifest.mjs'

const FIXTURE = path.join(import.meta.dir, '..', 'crates', 'beebeeb-updater', 'tests', 'fixtures', 'native-manifest-shape.json')
const NAME = 'Beebeeb_0.9.0_x64-setup.exe'
const b64 = (value: Buffer | string) => Buffer.from(value).toString('base64')

/** A minisign key pair made for this run (none is committed), signing in the form Tauri's signer writes. */
function testKey() {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(12)
  const keyId = Buffer.from('0102030405060708', 'hex')
  const pubkey = b64(`untrusted comment: minisign public key\n${b64(Buffer.concat([Buffer.from('Ed'), keyId, raw]))}\n`)
  const sigFor = (bytes: Buffer, file: string) => {
    const signature = sign(null, createHash('blake2b512').update(bytes).digest(), privateKey)
    const comment = `timestamp:1800000000\tfile:${file}`
    const global = sign(null, Buffer.concat([signature, Buffer.from(comment)]), privateKey)
    const line = b64(Buffer.concat([Buffer.from('ED'), keyId, signature]))
    return b64(`untrusted comment: signature from tauri secret key\n${line}\ntrusted comment: ${comment}\n${b64(global)}\n`)
  }
  return { pubkey, sigFor }
}

const key = testKey()
const INSTALLER = Buffer.alloc(9_000_001, 7)
const honest = {
  version: '0.9.0',
  channel: 'beta',
  installerName: NAME,
  installerBytes: INSTALLER,
  signature: key.sigFor(INSTALLER, NAME),
  pubkey: key.pubkey,
  notes: '# Beebeeb Desktop 0.9.0 - native\n',
  date: '2026-11-20T10:00:00Z',
}

describe('native channel manifest', () => {
  test('writes the shape the updater crate reads', () => {
    // The signature differs per run (a fresh key); everything else, key order included, is the fixture's bytes.
    const fixture = fs.readFileSync(FIXTURE, 'utf8')
    const manifest = buildManifest(honest)
    for (const platform of Object.values(manifest.platforms)) platform.signature = JSON.parse(fixture).platforms['windows-x86_64'].signature
    expect(`${JSON.stringify(manifest, null, 2)}\n`).toBe(fixture)
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
    expect(refusal({ version: '0.8.25', installerName: 'Beebeeb_0.8.25_x64-setup.exe', notes: '0.8.25' })).toContain('older than the first native release')
    expect(refusal({ channel: 'nightly' })).toContain('alpha, beta or stable')
    expect(refusal({ installerName: 'Beebeeb Test_0.9.0_x64-setup.exe' })).toContain('must be named')
    expect(refusal({ installerBytes: Buffer.alloc(0) })).toContain('empty')
    expect(refusal({ signature: key.sigFor(INSTALLER, 'Beebeeb_0.8.22_x64-setup.exe') })).toContain("signs 'Beebeeb_0.8.22_x64-setup.exe'")
    // A truncated download of the release asset, one changed byte, and another key's signature of the right file.
    expect(refusal({ installerBytes: INSTALLER.subarray(0, 9_000_000) })).toContain('not the file its signature signs')
    const changed = Buffer.from(INSTALLER)
    changed[4_000_000] ^= 1
    expect(refusal({ installerBytes: changed })).toContain('not the file its signature signs')
    expect(refusal({ signature: testKey().sigFor(INSTALLER, NAME) })).toContain('not the file its signature signs')
    expect(refusal({ signature: 'PGh0bWw+' })).toContain("signs 'null'")
    expect(refusal({ date: 'yesterday' })).toContain('must look like')
    expect(refusal({ notes: '# Beebeeb Desktop 0.8.9' })).toContain('do not name 0.9.0')
  })

  test('reads versions and signed file names strictly', () => {
    expect(parseVersion('0.10.2')).toEqual([0, 10, 2])
    expect(parseVersion('01.0.0')).toBeNull()
    expect(signedFileName(key.sigFor(INSTALLER, 'a.exe'))).toBe('a.exe')
    expect(signedFileName('')).toBeNull()
    expect(verifyMinisign(INSTALLER, honest.signature, key.pubkey)).toBe(true)
    expect(verifyMinisign(INSTALLER, honest.signature, 'AAAA')).toBe(false)
  })
})
