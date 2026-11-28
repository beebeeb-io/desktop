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
//     --sig <setup.exe.sig> --notes <RELEASE_NOTES.md> --pubkey crates/beebeeb-updater/keys/updater-v2.pub \
//     [--date 2026-11-20T10:00:00Z] [--existing <manifest.json>] --out <stable.json>
//   A Mac publish gives --mac-artifact <Beebeeb_0.9.0_aarch64.app.tar.gz> --mac-sig <file.sig> instead of
//   --installer/--sig and writes a `darwin-aarch64` entry. --existing is the manifest already published for the same
//   product, channel and version: the other platform's entries are kept, so neither publish drops the other's.
//   node scripts/native-channel-manifest.mjs --verify <file> --sig <file.sig> --signed-name <name> --pubkey <.pub>
//
// The installer's bytes are verified against the signature and the native line's public key (--pubkey, the file
// the updater crate bakes in) before anything is written: a truncated or replaced file, or one signed by the
// 0.8.x key, never gets a manifest, and so is never published. `--verify` is the same check for any signed file
// (the workflow runs it on the manifest it has just signed): right key, right bytes, right signed name.
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

function compareVersions(a, b) {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1
  return 0
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

const WINDOWS_TARGETS = ['windows-x86_64', 'windows-x86_64-nsis']
export const DARWIN_TARGET = 'darwin-aarch64'

/** The artefact each platform publishes, by its name for a version. */
export const artifactName = (kind, version) => (kind === 'darwin' ? `Beebeeb_${version}_aarch64.app.tar.gz` : `Beebeeb_${version}_x64-setup.exe`)

/**
 * Checks one platform's artefact and returns its manifest entry; throws an Error whose message names the bad input.
 * Everything but the platform is shared: version, channel, date, notes, name, size, signature, key.
 */
function platformEntry(kind, { version, channel, installerName, installerBytes, signature, pubkey, notes, date }) {
  const parsed = parseVersion(version)
  if (!parsed) throw new Error(`version '${version}' must be plain semver like 0.9.0`)
  if (!atLeast(parsed, NATIVE_FLOOR)) throw new Error(`version ${version} is older than the first native release 0.9.0: it belongs in release.yml's manifests`)
  if (!Object.hasOwn(CHANNEL_FILES, channel)) throw new Error(`channel '${channel}' must be alpha, beta or stable`)
  const expectedName = artifactName(kind, version)
  if (installerName !== expectedName) throw new Error(`the ${kind === 'darwin' ? 'Mac artefact' : 'installer'} must be named ${expectedName}, got ${installerName}`)
  const installerSize = installerBytes.length
  if (installerSize <= 0) throw new Error(`the ${kind === 'darwin' ? 'Mac artefact' : 'installer'} is empty`)
  const signed = signedFileName(signature)
  if (signed !== expectedName) throw new Error(`the signature signs '${signed}', not ${expectedName}`)
  if (!verifyMinisign(installerBytes, signature, pubkey)) throw new Error('the installer is not the file its signature signs (or another key signed it)')
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(date)) throw new Error(`date '${date}' must look like 2026-11-20T10:00:00Z`)
  if (!notes.includes(version)) throw new Error(`the release notes do not name ${version}`)
  return { url: `${RELEASE_HOST}/desktop/artifacts/${version}/${expectedName}`, signature: signature.trim(), size: installerSize }
}

/**
 * The entries of `existing` (a manifest already published for this release) that belong to the OTHER platform, so a
 * Windows publish never drops the Mac entry and a Mac publish never drops the Windows ones. Only the same product,
 * channel and version: an entry of another version names another file and the updater would refuse it, so it is
 * not carried over. `ownTargets` are the targets being written now: never kept from the old file.
 */
export function keptEntries(existing, { version, channel }, ownTargets, { sameVersionOnly = false } = {}) {
  if (!existing) return {}
  if (existing.product !== 'desktop' || existing.channel !== channel) throw new Error(`the existing manifest is for ${existing.product}/${existing.channel}, not desktop/${channel}`)
  if (existing.version !== version) {
    const old = parseVersion(existing.version)
    const next = parseVersion(version)
    // Never roll a channel back, whichever platform publishes.
    if (old && next && compareVersions(next, old) < 0) throw new Error(`the channel already publishes ${existing.version}: refusing to publish the lower ${version}`)
    // A Mac publish for another version would replace the manifest with a darwin-only one and strand Windows.
    if (sameVersionOnly) throw new Error(`the channel publishes ${existing.version}, not ${version}: publish the Windows build of ${version} first, a Mac-only manifest would drop the Windows entries`)
    return {}
  }
  return Object.fromEntries(Object.entries(existing.platforms ?? {}).filter(([target]) => !ownTargets.includes(target)))
}

function manifestOf(args, platforms, options) {
  const { version, channel, notes, date, existing } = args
  const own = Object.keys(platforms)
  return {
    version,
    notes,
    pub_date: date,
    product: 'desktop',
    channel,
    platforms: { ...keptEntries(existing, { version, channel }, own, options), ...platforms },
  }
}

/** Builds the Windows manifest object (keeping an `existing` manifest's Mac entry), or throws an Error naming the bad input. */
export function buildManifest(args) {
  const platform = platformEntry('windows', args)
  return manifestOf(args, Object.fromEntries(WINDOWS_TARGETS.map((target) => [target, platform])))
}

/** Builds the manifest object of a Mac publish: a `darwin-aarch64` entry (url, signature, size), keeping an `existing` manifest's Windows entries. */
export function buildDarwinManifest(args) {
  return manifestOf(args, { [DARWIN_TARGET]: platformEntry('darwin', args) }, { sameVersionOnly: true })
}

function main(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) throw new Error(`bad argument '${argv[i]}'`)
    args[argv[i].slice(2)] = argv[i + 1]
  }
  if (args.verify) {
    for (const required of ['sig', 'signed-name', 'pubkey']) if (!args[required]) throw new Error(`--${required} is required`)
    const signature = readFileSync(args.sig, 'utf8')
    if (signedFileName(signature) !== args['signed-name']) {
      throw new Error(`the signature signs '${signedFileName(signature)}', not ${args['signed-name']}`)
    }
    if (!verifyMinisign(readFileSync(args.verify), signature, readFileSync(args.pubkey, 'utf8'))) {
      throw new Error(`${args.verify} is not the file its signature signs (or another key signed it)`)
    }
    console.log(`verified ${args.verify}: signed as ${args['signed-name']} by the key in ${args.pubkey}`)
    return
  }
  // A Mac publish names `--mac-artifact`/`--mac-sig`; a Windows one `--installer`/`--sig`.
  const mac = args['mac-artifact'] !== undefined
  const [file, sigFile] = mac ? ['mac-artifact', 'mac-sig'] : ['installer', 'sig']
  for (const required of ['version', 'channel', file, sigFile, 'notes', 'pubkey', 'out']) {
    if (!args[required]) throw new Error(`--${required} is required`)
  }
  const manifest = (mac ? buildDarwinManifest : buildManifest)({
    version: args.version,
    channel: args.channel,
    installerName: basename(args[file]),
    installerBytes: readFileSync(args[file]),
    signature: readFileSync(args[sigFile], 'utf8'),
    pubkey: readFileSync(args.pubkey, 'utf8'),
    notes: readFileSync(args.notes, 'utf8'),
    date: args.date ?? new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
    existing: args.existing ? JSON.parse(readFileSync(args.existing, 'utf8')) : undefined,
  })
  writeFileSync(args.out, `${JSON.stringify(manifest, null, 2)}\n`)
  console.log(`wrote ${args.out}: ${manifest.version} on ${manifest.channel}, platforms ${Object.keys(manifest.platforms).join(', ')}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2))
  } catch (error) {
    console.error(`native-channel-manifest: ${error.message}`)
    process.exit(1)
  }
}
