// The v2 channel manifest a native Windows release publishes (scripts/native-channel-manifest.mjs). The fixture it
// must reproduce byte for byte is the one the updater crate parses in its own tests
// (crates/beebeeb-updater/src/manifest.rs), so the script and the crate cannot drift apart unnoticed.
import { describe, expect, test } from 'bun:test'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { buildManifest, parseVersion, signedFileName, verifyMinisign } from '../native-channel-manifest.mjs'

const FIXTURE = path.join(import.meta.dir, '..', '..', 'crates', 'beebeeb-updater', 'tests', 'fixtures', 'native-manifest-shape.json')
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

// The native line has its own key (crates/beebeeb-updater/keys/updater-v2.pub). The pipeline reads that one file:
// the command line takes it as --pubkey, and the key guard as --pubkey-file.
describe('the native line is signed by the v2 key only', () => {
  const V2_PUB = path.join(import.meta.dir, '..', '..', 'crates', 'beebeeb-updater', 'keys', 'updater-v2.pub')
  const script = (name: string) => path.join(import.meta.dir, '..', name)
  const run = (args: string[]) => Bun.spawnSync(['node', ...args], { stdout: 'pipe', stderr: 'pipe' })
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'native-manifest-'))
  const write = (name: string, content: Buffer | string) => {
    const file = path.join(tmp, name)
    fs.writeFileSync(file, content)
    return file
  }

  test('the committed v2 key is a minisign public key and not the 0.8.x one', () => {
    const v2 = fs.readFileSync(V2_PUB, 'utf8').trim()
    const old = JSON.parse(fs.readFileSync(path.join(import.meta.dir, '..', '..', 'src-tauri', 'tauri.conf.json'), 'utf8')).plugins.updater.pubkey
    expect(v2).not.toBe(old)
    expect(Buffer.from(v2, 'base64').toString('utf8')).toStartWith('untrusted comment: minisign public key: FED25848C41F2348\n')
    // Pinned in a second place (and a third: release-native-windows.yml): replacing the committed key alone fails.
    expect(createHash('sha256').update(fs.readFileSync(V2_PUB)).digest('hex')).toBe('8e8ab5cb63a9e1e467135a79dabb9dd9f58895f547a078fd7d61a24244ad3284')
  })

  test('--verify accepts the right key, bytes and name, and refuses each single lie', () => {
    const manifest = Buffer.from('{"version":"0.9.0"}\n')
    const file = write('stable.json', manifest)
    const pub = write('test.pub', key.pubkey)
    const sig = write('stable.json.sig', key.sigFor(manifest, 'stable.json'))
    const verify = (overrides: Record<string, string>) => {
      const args = { verify: file, sig, 'signed-name': 'stable.json', pubkey: pub, ...overrides }
      return run([script('native-channel-manifest.mjs'), ...Object.entries(args).flatMap(([k, v]) => [`--${k}`, v])]).exitCode
    }
    expect(verify({})).toBe(0)
    expect(verify({ 'signed-name': 'beta.json' })).toBe(1)
    expect(verify({ verify: write('other.json', '{"version":"0.9.1"}\n') })).toBe(1)
    expect(verify({ pubkey: write('other.pub', testKey().pubkey) })).toBe(1)
    // A manifest signed by a test key is not signed by the committed v2 key.
    expect(verify({ pubkey: V2_PUB })).toBe(1)
  })

  test('the key guard, pointed at the v2 key, refuses a signature the 0.8.x key made', () => {
    const published = JSON.parse(fs.readFileSync(path.join(import.meta.dir, '..', '..', 'crates', 'beebeeb-updater', 'tests', 'fixtures', 'desktop-latest-0.8.22.json'), 'utf8'))
    const oldSig = write('old.sig', published.platforms['windows-x86_64-nsis'].signature)
    const guard = script('check-updater-signature-key.mjs')
    expect(run([guard, '--sig', oldSig]).exitCode).toBe(0) // the control: it IS the 0.8.x key's
    const refused = run([guard, '--pubkey-file', V2_PUB, '--sig', oldSig])
    expect(refused.exitCode).toBe(1)
    expect(refused.stdout.toString()).toContain('the expected key is FED25848C41F2348')
  })
})
