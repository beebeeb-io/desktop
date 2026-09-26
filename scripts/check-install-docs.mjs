#!/usr/bin/env node
// Install/verify docs check (flow 7, issue 9).
//
// Asserts that a person who lands on this repo can install the desktop app and
// verify what they downloaded, using nothing but the README:
//
//   1. CONTRIBUTING.md exists at the repo root.
//   2. README.md has Install, Verify-your-download and Updating sections, with
//      the per-OS steps that trip people up (SmartScreen, AppImage chmod).
//   3. README.md contains a `minisign -Vm <file> -P <key>` command whose full
//      base64 public key is the key that ACTUALLY signs the current GitHub
//      "latest" desktop release. This is proven cryptographically, not by
//      string-matching a key ID: the script downloads one real `.sig` from the
//      latest release, decodes it, and verifies its minisign global signature
//      (ed25519 over signature || trusted comment) with each key found in the
//      README. At least one README key must verify.
//   4. README.md no longer carries the false conflict claim
//      ("the older version is renamed `file (Device, HH:MM).ext`").
//
// Network: needs read-only HTTPS to api.github.com + github.com. It fails
// closed if it cannot reach them — a check that cannot prove anything is a red.
//
// Usage: node scripts/check-install-docs.mjs   (or: bun run check:docs)
//        README_PATH=/tmp/x.md node scripts/check-install-docs.mjs  (self-test)

import { createPublicKey, verify } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const readmePath = process.env.README_PATH ?? path.join(repoRoot, 'README.md')
const contributingPath = process.env.CONTRIBUTING_PATH ?? path.join(repoRoot, 'CONTRIBUTING.md')
const releaseRepo = process.env.DESKTOP_RELEASE_REPO ?? 'beebeeb-io/desktop'
const ed25519SpkiPrefix = Buffer.from('302a300506032b6570032100', 'hex')

const failures = []
let passes = 0
function check(ok, label) {
  if (ok) {
    passes += 1
    console.log(`ok   ${label}`)
  } else {
    failures.push(label)
    console.log(`FAIL ${label}`)
  }
}

// ── 1. CONTRIBUTING.md ────────────────────────────────────────────────────────
check(fs.existsSync(contributingPath), 'CONTRIBUTING.md exists at the repo root')

// ── 2. README sections ────────────────────────────────────────────────────────
const readme = fs.readFileSync(readmePath, 'utf8')
const headings = [...readme.matchAll(/^#{2,3} +(.+)$/gm)].map((m) => m[1].trim().toLowerCase())
check(headings.some((h) => h.startsWith('install')), 'README has an "Install" section')
check(headings.some((h) => h.startsWith('verify your download')), 'README has a "Verify your download" section')
check(headings.some((h) => h.startsWith('updating')), 'README has an "Updating" section')
check(/smartscreen/i.test(readme) && /run anyway/i.test(readme), 'README walks through the Windows SmartScreen "Run anyway" step')
check(/chmod \+x [^\n]*\.AppImage/.test(readme), 'README tells Linux users to chmod +x the AppImage')
check(/\.deb\b/.test(readme) && /\.rpm\b/.test(readme) && /\.msi\b/.test(readme) && /setup\.exe/.test(readme),
  'README names every published installer type (.deb, .rpm, .msi, setup.exe)')
check(!/older version is renamed `?file \(Device, HH:MM\)/.test(readme),
  'README does not repeat the false "older version is renamed file (Device, HH:MM)" conflict claim')
// The conflict description must match what the runner actually does: if the
// runner calls sweep_auto_resolutions (24 h auto Keep Both), the README must
// say so, and must not claim a conflict waits for the user indefinitely.
const runnerPath = process.env.RUNNER_PATH ?? path.join(repoRoot, 'src-tauri', 'src', 'runner.rs')
const runnerSweeps = /^\s*sweep_auto_resolutions\(/m.test(fs.readFileSync(runnerPath, 'utf8'))
check(!runnerSweeps || /24 hours[^\n]*Keep Both/i.test(readme),
  `README describes the 24 h automatic Keep Both (runner calls sweep_auto_resolutions: ${runnerSweeps})`)
check(!runnerSweeps || !/nothing is overwritten until you choose/i.test(readme),
  'README does not claim a conflict waits for the user indefinitely')

// ── 3. The README key verifies the latest release's real signature ───────────
function decodeMinisignPublicKey(b64) {
  const raw = Buffer.from(b64, 'base64')
  if (raw.length !== 42 || raw.subarray(0, 2).toString('latin1') !== 'Ed') return null
  return { keyId: Buffer.from(raw.subarray(2, 10)).reverse().toString('hex').toUpperCase(), pk: raw.subarray(10) }
}

function parseTauriSig(text) {
  // Tauri's .sig is base64 of a whole minisign signature file.
  const lines = Buffer.from(text.trim(), 'base64').toString('utf8').split('\n')
  const sigLine = Buffer.from(lines[1] ?? '', 'base64')
  const trusted = (lines[2] ?? '').replace(/^trusted comment: /, '')
  const globalSig = Buffer.from(lines[3] ?? '', 'base64')
  if (sigLine.length !== 74 || globalSig.length !== 64) throw new Error('malformed .sig (not a minisign signature)')
  return {
    keyId: Buffer.from(sigLine.subarray(2, 10)).reverse().toString('hex').toUpperCase(),
    signature: sigLine.subarray(10),
    trusted,
    globalSig,
  }
}

function globalSigVerifies(pk, sig) {
  const key = createPublicKey({ key: Buffer.concat([ed25519SpkiPrefix, pk]), format: 'der', type: 'spki' })
  return verify(null, Buffer.concat([sig.signature, Buffer.from(sig.trusted, 'utf8')]), key, sig.globalSig)
}

// Every `minisign -Vm <file> -P <key>` invocation in the README.
const commandKeys = [...readme.matchAll(/minisign\s+-Vm\s+\S+\s+-P\s+['"]?(RW[A-Za-z0-9+/]{54})['"]?/g)].map((m) => m[1])
check(commandKeys.length > 0, 'README contains a `minisign -Vm <file> -P <full base64 key>` command')

try {
  const headers = { 'User-Agent': 'beebeeb-desktop-docs-check', Accept: 'application/vnd.github+json' }
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  const res = await fetch(`https://api.github.com/repos/${releaseRepo}/releases/latest`, { headers })
  if (!res.ok) throw new Error(`GitHub latest-release lookup returned HTTP ${res.status}`)
  const release = await res.json()
  const sigAsset = release.assets.find((a) => a.name.endsWith('.sig'))
  if (!sigAsset) throw new Error(`latest release ${release.tag_name} has no .sig asset`)
  const sigRes = await fetch(sigAsset.browser_download_url, { headers: { 'User-Agent': headers['User-Agent'] } })
  if (!sigRes.ok) throw new Error(`downloading ${sigAsset.name} returned HTTP ${sigRes.status}`)
  const sig = parseTauriSig(await sigRes.text())
  console.log(`info latest release ${release.tag_name}: ${sigAsset.name} is signed by key ID ${sig.keyId}`)

  const verifying = commandKeys.filter((b64) => {
    const key = decodeMinisignPublicKey(b64)
    return key && key.keyId === sig.keyId && globalSigVerifies(key.pk, sig)
  })
  check(verifying.length > 0,
    `README's minisign command key verifies the real signature of ${release.tag_name} (signing key ID ${sig.keyId})`)
  check(readme.includes(sig.keyId), `README names the signing key ID ${sig.keyId} next to the key`)
  // The updater key baked into the app must be the key that signs "latest",
  // otherwise in-app updates are broken and the README's Updating section lies.
  const conf = JSON.parse(fs.readFileSync(path.join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'))
  const bakedB64 = Buffer.from(conf.plugins.updater.pubkey, 'base64').toString('utf8').split('\n')[1] ?? ''
  const baked = decodeMinisignPublicKey(bakedB64.trim())
  check(Boolean(baked) && baked.keyId === sig.keyId && commandKeys.includes(bakedB64.trim()),
    `README's minisign command uses the app's baked updater key (${baked?.keyId ?? 'unparseable'}), which signs ${release.tag_name}`)
} catch (error) {
  check(false, `could not prove the README key against the latest release: ${error.message}`)
}

console.log(`\ninstall-docs check: ${passes} passed, ${failures.length} failed`)
process.exit(failures.length === 0 && passes > 0 ? 0 : 1)
