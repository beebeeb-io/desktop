#!/usr/bin/env node

// Fails when an updater signature was made by a different minisign key than the
// one baked into src-tauri/tauri.conf.json (plugins.updater.pubkey).
//
// Why this exists: task-1240 baked the new updater key C6FADFD59D732197 into
// 0.3.0, but CI kept signing with the old key 545D7BA77EDEA7E1 through 0.8.3.
// Every install >= 0.3.0 trusts only C6FA, so it rejected every published
// update — silently, for months. This guard makes that state a red release.
//
// Usage:
//   node scripts/check-updater-signature-key.mjs [--config <tauri.conf.json>]
//        (--sig <file.sig>... | --manifest <path-or-https-url>)
//        [--allow-key-transition]
//
// --allow-key-transition is ONLY for a deliberate transition release N (see
// docs/RELEASING.md "Updater Signing Key Rotation"): it reports mismatches as a
// warning instead of failing. Every other release must pass without it.

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

function decodeBase64Strict(value, what) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9+/]+={0,2}$/.test(value.trim())) {
    throw new Error(`${what} is not base64`)
  }
  return Buffer.from(value.trim(), 'base64')
}

function hexKeyId(bytes) {
  // minisign stores the key id little-endian and displays it as a big-endian u64.
  return Buffer.from(bytes).reverse().toString('hex').toUpperCase()
}

// Tauri stores both the pubkey and the .sig as base64 of the full minisign file
// text. The second line of that text is base64 of: 2-byte algorithm, 8-byte key
// id, then key or signature material.
function minisignPayload(tauriValue, what, algorithms, minLength) {
  const text = decodeBase64Strict(tauriValue, what).toString('utf8')
  const lines = text.split(/\r?\n/)
  if (lines.length < 2 || !lines[0].startsWith('untrusted comment:')) {
    throw new Error(`${what} is not a minisign file (missing untrusted comment line)`)
  }
  const payload = decodeBase64Strict(lines[1], `${what} payload line`)
  const algorithm = payload.subarray(0, 2).toString('latin1')
  if (payload.length < minLength || !algorithms.includes(algorithm)) {
    throw new Error(`${what} has an unexpected minisign payload (algorithm '${algorithm}', ${payload.length} bytes)`)
  }
  return payload
}

export function keyIdOfPubkey(tauriPubkey) {
  return hexKeyId(minisignPayload(tauriPubkey, 'updater pubkey', ['Ed'], 42).subarray(2, 10))
}

export function keyIdOfSignature(tauriSignature) {
  return hexKeyId(minisignPayload(tauriSignature, 'updater signature', ['Ed', 'ED'], 74).subarray(2, 10))
}

function compare(entries, tauriPubkey) {
  if (entries.length === 0) {
    throw new Error('no platform signatures to check')
  }
  const expectedKeyId = keyIdOfPubkey(tauriPubkey)
  const mismatches = []
  for (const { target, signature } of entries) {
    const signatureKeyId = keyIdOfSignature(signature)
    if (signatureKeyId !== expectedKeyId) {
      mismatches.push({ target, signatureKeyId, expectedKeyId })
    }
  }
  return { expectedKeyId, checked: entries.length, mismatches }
}

export function checkManifestSignatures(manifest, tauriPubkey) {
  const platforms = manifest && typeof manifest === 'object' ? manifest.platforms ?? {} : {}
  return compare(
    Object.entries(platforms).map(([target, entry]) => ({ target, signature: entry?.signature })),
    tauriPubkey,
  )
}

export function checkSignatureFiles(files, tauriPubkey) {
  return compare(
    files.map((file) => ({ target: path.basename(file), signature: fs.readFileSync(file, 'utf8') })),
    tauriPubkey,
  )
}

async function loadManifest(source) {
  if (/^https:\/\//.test(source)) {
    const response = await fetch(source, { cache: 'no-store' })
    if (!response.ok) {
      throw new Error(`GET ${source} -> HTTP ${response.status}`)
    }
    return response.json()
  }
  return JSON.parse(fs.readFileSync(source, 'utf8'))
}

async function main(argv) {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  let config = path.join(repoRoot, 'src-tauri', 'tauri.conf.json')
  let manifestSource = null
  let allowTransition = false
  const sigFiles = []

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--config') config = argv[++i]
    else if (arg === '--manifest') manifestSource = argv[++i]
    else if (arg === '--allow-key-transition') allowTransition = true
    else if (arg === '--sig') {
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) sigFiles.push(argv[++i])
    } else throw new Error(`unknown argument: ${arg}`)
  }
  if (Boolean(manifestSource) === sigFiles.length > 0) {
    throw new Error('pass exactly one of --manifest <path-or-url> or --sig <file.sig>...')
  }

  const pubkey = JSON.parse(fs.readFileSync(config, 'utf8'))?.plugins?.updater?.pubkey
  if (!pubkey) throw new Error(`${config} has no plugins.updater.pubkey`)

  const result = manifestSource
    ? checkManifestSignatures(await loadManifest(manifestSource), pubkey)
    : checkSignatureFiles(sigFiles, pubkey)

  const subject = manifestSource ?? `${sigFiles.length} .sig file(s)`
  if (result.mismatches.length === 0) {
    console.log(
      `updater signature key OK: ${result.checked} of ${result.checked} signatures in ${subject} use baked key ${result.expectedKeyId}`,
    )
    return 0
  }

  const level = allowTransition ? 'warning' : 'error'
  for (const m of result.mismatches) {
    console.log(
      `::${level}::Updater signature for ${m.target} was made by key ${m.signatureKeyId}, but tauri.conf.json bakes in ${m.expectedKeyId}. Installs of this version will reject every update signed like this.`,
    )
  }
  console.log(
    `updater signature key MISMATCH: ${result.mismatches.length} of ${result.checked} signatures in ${subject} do not use baked key ${result.expectedKeyId}`,
  )
  if (allowTransition) {
    console.log('::warning::--allow-key-transition set: accepted ONLY because this is a deliberate transition release N (docs/RELEASING.md).')
    return 0
  }
  return 1
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      console.error(`::error::${error.message}`)
      process.exit(2)
    },
  )
}
