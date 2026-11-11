#!/usr/bin/env node
// Writes the channel manifest of a NATIVE Windows release (0.9.0 and later): `desktop/v2/<channel>.json`.
//
// The shape is the one 0.8.x's updater reads (version, notes, pub_date, platforms.<target>.{url,signature}) plus
// three additions it ignores and the native updater requires (crates/beebeeb-updater/src/manifest.rs): `product`,
// `channel` and `platforms.<target>.size`. The 0.8.x manifests (`desktop/latest.json`, `beta.json`, `alpha.json`)
// are written by release.yml and are not touched here: a native release never appears in them.
//
// Usage:
//   node scripts/native-channel-manifest.mjs --version 0.9.0 --channel stable --installer <setup.exe> \
//     --sig <setup.exe.sig> --notes <RELEASE_NOTES.md> --config src-tauri/tauri.conf.json \
//     [--date 2026-11-20T10:00:00Z] --out <stable.json>
//
// The installer's bytes are verified against the signature and the baked updater public key (--config) before
// anything is written: a truncated or replaced file never gets a manifest, and so is never published.
//
// The installer is published at https://releases.beebeeb.io/desktop/artifacts/<version>/<file name>: the one host
// the native updater accepts. Exits 1 with one line on any input it does not like; writes nothing in that case.

import { createHash, createPublicKey, verify } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { basename } from 'node:path'
import { pathToFileURL } from 'node:url'

export const RELEASE_HOST = 'https://releases.beebeeb.io'
export const CHANNEL_FILES = { alpha: 'alpha.json', beta: 'beta.json', stable: 'stable.json' }
/** The first native release: nothing older is published to a v2 manifest. */
export const NATIVE_FLOOR = [0, 9, 0]

export function parseVersion(text) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(text ?? '')
  return match ? match.slice(1).map(Number) : null
}

function atLeast(version, floor) {
  for (let i = 0; i < 3; i++) if (version[i] !== floor[i]) return version[i] > floor[i]
  return true
}

/** The signed file name in a Tauri/minisign `.sig` (base64 of the signature text), or null. */
export function signedFileName(sigBase64) {
  let text
  try {
    text = Buffer.from(sigBase64.trim(), 'base64').toString('utf8')
  } catch {
    return null
  }
  const lines = text.split('\n')
  if (!lines[0]?.startsWith('untrusted comment: ') || !lines[2]?.startsWith('trusted comment: ')) return null
  const field = lines[2].slice('trusted comment: '.length).split('\t').find((part) => part.startsWith('file:'))
  return field ? field.slice('file:'.length) : null
}

/**
 * True when `sigBase64` (a Tauri/minisign `.sig`) is a pre-hashed signature of `bytes` by the key in
 * `pubkeyBase64` (the `plugins.updater.pubkey` form), trusted comment included. The check Tauri's updater and
 * crates/beebeeb-updater make, for the release pipeline.
 */
export function verifyMinisign(bytes, sigBase64, pubkeyBase64) {
  try {
    const pubLines = Buffer.from(pubkeyBase64.trim(), 'base64').toString('utf8').split('\n')
    const pub = Buffer.from(pubLines[1], 'base64') // "Ed" + 8 bytes key id + 32 bytes key
    const lines = Buffer.from(sigBase64.trim(), 'base64').toString('utf8').split('\n')
    const sig = Buffer.from(lines[1], 'base64') // "ED" (pre-hashed) + 8 bytes key id + 64 bytes signature
    const global = Buffer.from(lines[3], 'base64')
    if (pub.length !== 42 || sig.length !== 74 || global.length !== 64) return false
    if (pub.subarray(0, 2).toString() !== 'Ed' || sig.subarray(0, 2).toString() !== 'ED') return false
    if (!pub.subarray(2, 10).equals(sig.subarray(2, 10))) return false
    if (!lines[2].startsWith('trusted comment: ')) return false
    const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), pub.subarray(10)]), format: 'der', type: 'spki' })
    const digest = createHash('blake2b512').update(bytes).digest()
    const signature = sig.subarray(10)
    const comment = Buffer.from(lines[2].slice('trusted comment: '.length), 'utf8')
    return verify(null, digest, key, signature) && verify(null, Buffer.concat([signature, comment]), key, global)
  } catch {
    return false
  }
}

/** Builds the manifest object, or throws an Error whose message names the bad input. */
export function buildManifest({ version, channel, installerName, installerBytes, signature, pubkey, notes, date }) {
  const parsed = parseVersion(version)
  if (!parsed) throw new Error(`version '${version}' must be plain semver like 0.9.0`)
  if (!atLeast(parsed, NATIVE_FLOOR)) throw new Error(`version ${version} is older than the first native release 0.9.0: it belongs in release.yml's manifests`)
  if (!Object.hasOwn(CHANNEL_FILES, channel)) throw new Error(`channel '${channel}' must be alpha, beta or stable`)
  const expectedName = `Beebeeb_${version}_x64-setup.exe`
  if (installerName !== expectedName) throw new Error(`the installer must be named ${expectedName}, got ${installerName}`)
  const installerSize = installerBytes.length
  if (installerSize <= 0) throw new Error('the installer is empty')
  const signed = signedFileName(signature)
  if (signed !== expectedName) throw new Error(`the signature signs '${signed}', not ${expectedName}`)
  if (!verifyMinisign(installerBytes, signature, pubkey)) throw new Error('the installer is not the file its signature signs (or another key signed it)')
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(date)) throw new Error(`date '${date}' must look like 2026-11-20T10:00:00Z`)
  if (!notes.includes(version)) throw new Error(`the release notes do not name ${version}`)
  const platform = { url: `${RELEASE_HOST}/desktop/artifacts/${version}/${expectedName}`, signature: signature.trim(), size: installerSize }
  return {
    version,
    notes,
    pub_date: date,
    product: 'desktop',
    channel,
    platforms: { 'windows-x86_64': platform, 'windows-x86_64-nsis': platform },
  }
}

function main(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) throw new Error(`bad argument '${argv[i]}'`)
    args[argv[i].slice(2)] = argv[i + 1]
  }
  for (const required of ['version', 'channel', 'installer', 'sig', 'notes', 'config', 'out']) {
    if (!args[required]) throw new Error(`--${required} is required`)
  }
  const manifest = buildManifest({
    version: args.version,
    channel: args.channel,
    installerName: basename(args.installer),
    installerBytes: readFileSync(args.installer),
    signature: readFileSync(args.sig, 'utf8'),
    pubkey: JSON.parse(readFileSync(args.config, 'utf8'))?.plugins?.updater?.pubkey ?? '',
    notes: readFileSync(args.notes, 'utf8'),
    date: args.date ?? new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
  })
  writeFileSync(args.out, `${JSON.stringify(manifest, null, 2)}\n`)
  console.log(`wrote ${args.out}: ${manifest.version} on ${manifest.channel}, ${manifest.platforms['windows-x86_64-nsis'].size} bytes`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2))
  } catch (error) {
    console.error(`native-channel-manifest: ${error.message}`)
    process.exit(1)
  }
}
