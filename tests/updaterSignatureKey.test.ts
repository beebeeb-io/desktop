import { describe, expect, test } from 'bun:test'
import { generateKeyPairSync, randomBytes } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import {
  checkManifestSignatures,
  keyIdOfPubkey,
  keyIdOfSignature,
} from '../scripts/check-updater-signature-key.mjs'

// Builds minisign-shaped fixtures (the Tauri updater format: base64 of the
// full minisign file text). Only the key-id bytes matter to the guard, so the
// signature bytes are random — this test never touches a real signing key.
function keyIdBytes(): Buffer {
  return randomBytes(8)
}

function hexId(id: Buffer): string {
  return Buffer.from(id).reverse().toString('hex').toUpperCase()
}

function tauriPubkey(id: Buffer): string {
  const { publicKey } = generateKeyPairSync('ed25519')
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32)
  const line = Buffer.concat([Buffer.from('Ed'), id, raw]).toString('base64')
  return Buffer.from(`untrusted comment: minisign public key: ${hexId(id)}\n${line}\n`).toString('base64')
}

function tauriSignature(id: Buffer): string {
  const line = Buffer.concat([Buffer.from('ED'), id, randomBytes(64)]).toString('base64')
  const text =
    'untrusted comment: signature from tauri secret key\n' +
    `${line}\n` +
    'trusted comment: timestamp:1700000000\tfile:Beebeeb_0.8.4_amd64.AppImage\n' +
    `${randomBytes(64).toString('base64')}\n`
  return Buffer.from(text).toString('base64')
}

function manifest(sigs: Record<string, string>) {
  return {
    version: '0.8.4',
    platforms: Object.fromEntries(
      Object.entries(sigs).map(([platform, signature]) => [platform, { url: `https://example.invalid/${platform}`, signature }]),
    ),
  }
}

describe('updater signature key guard', () => {
  test('reads the key id baked into the real tauri.conf.json pubkey', () => {
    const conf = JSON.parse(fs.readFileSync(path.join(import.meta.dir, '..', 'src-tauri', 'tauri.conf.json'), 'utf8'))
    expect(keyIdOfPubkey(conf.plugins.updater.pubkey)).toBe('C6FADFD59D732197')
  })

  test('decodes pubkey and signature key ids in minisign display order', () => {
    const id = keyIdBytes()
    expect(keyIdOfPubkey(tauriPubkey(id))).toBe(hexId(id))
    expect(keyIdOfSignature(tauriSignature(id))).toBe(hexId(id))
  })

  test('passes when every platform is signed by the baked key', () => {
    const id = keyIdBytes()
    const result = checkManifestSignatures(
      manifest({ 'linux-x86_64': tauriSignature(id), 'windows-x86_64-nsis': tauriSignature(id), 'windows-x86_64-msi': tauriSignature(id) }),
      tauriPubkey(id),
    )
    expect(result.checked).toBe(3)
    expect(result.mismatches).toEqual([])
  })

  test('fails when any platform is signed by a different key (the 0.3.0-0.8.3 failure)', () => {
    const baked = keyIdBytes()
    const ci = keyIdBytes()
    const result = checkManifestSignatures(
      manifest({ 'linux-x86_64': tauriSignature(baked), 'windows-x86_64-nsis': tauriSignature(ci) }),
      tauriPubkey(baked),
    )
    expect(result.checked).toBe(2)
    expect(result.mismatches).toEqual([
      { target: 'windows-x86_64-nsis', signatureKeyId: hexId(ci), expectedKeyId: hexId(baked) },
    ])
  })

  test('a manifest with no platforms is a failure, not a vacuous pass', () => {
    expect(() => checkManifestSignatures({ version: '0.8.4', platforms: {} }, tauriPubkey(keyIdBytes()))).toThrow(
      /no platform signatures/,
    )
  })

  test('rejects a signature that is not a minisign blob', () => {
    expect(() => keyIdOfSignature(Buffer.from('not a signature').toString('base64'))).toThrow()
  })
})
