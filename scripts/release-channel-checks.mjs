#!/usr/bin/env node

// Release-channel honesty checks for Beebeeb Desktop.
//
// Build once, promote by manifest (docs/RELEASING.md): a release is created on one
// channel (alpha/beta/stable) and later promoted. Two things went wrong with
// desktop-v0.8.3 and these checks exist so they cannot silently happen again:
//
// 1. Its notes said "Existing desktop installs receive this release through the
//    in-app updater automatically" while it was only ever published to alpha.json.
//    Stable and beta installs never saw it. `notes` fails closed on that claim for
//    any non-stable initial channel.
// 2. GitHub marked it "Latest" (and beebeeb.io/download serves GitHub's latest)
//    while desktop/latest.json — the stable updater manifest — stayed on 0.8.2.
//    `delivery` compares the two live values and exits non-zero on drift.
//
// Usage:
//   node scripts/release-channel-checks.mjs notes <version> <channel> [RELEASE_NOTES.md]
//   node scripts/release-channel-checks.mjs delivery

import fs from 'node:fs'
import { fileURLToPath } from 'node:url'

export const CHANNELS = ['alpha', 'beta', 'stable']
export const STABLE_MANIFEST_URL = 'https://releases.beebeeb.io/desktop/latest.json'
export const LATEST_RELEASE_API = 'https://api.github.com/repos/beebeeb-io/desktop/releases/latest'

// An unconditional promise of automatic delivery to existing installs. Only the
// stable manifest reaches every existing install, so only a release whose first
// channel is stable may say it.
const AUTOMATIC_DELIVERY_CLAIM =
  /\b(?:existing|all)\b[^.\n]{0,80}\binstalls?\b[^.\n]{0,120}\bautomatically\b/i

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function validateReleaseNotes(notes, version, channel) {
  const errors = []

  if (!/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) {
    errors.push(`Desktop release version '${version}' must be plain semver like 0.2.1.`)
  }
  if (!CHANNELS.includes(channel)) {
    errors.push(`Desktop release channel '${channel}' must be alpha, beta, or stable.`)
  }

  const exactVersion = new RegExp(
    `(?<![0-9A-Za-z.-])${escapeRegExp(version)}(?![0-9A-Za-z-]|\\.[0-9A-Za-z])`,
  )
  if (!exactVersion.test(notes)) {
    errors.push(
      `RELEASE_NOTES.md must mention the exact release version ${version}. Update the title/body for this release and commit it before rerunning the workflow. See docs/RELEASING.md.`,
    )
  }

  if (channel !== 'stable') {
    const claim = notes.match(AUTOMATIC_DELIVERY_CLAIM)
    if (claim) {
      errors.push(
        `RELEASE_NOTES.md claims automatic delivery to existing installs ("${claim[0].trim()}"), but this release is published to the ${channel} channel only — stable installs will not see it until it is promoted. Use the channel-aware Install / Update wording from docs/RELEASING.md.`,
      )
    }
  }

  return errors
}

export function normalizeDesktopTag(tag) {
  return String(tag ?? '').replace(/^desktop-v/, '')
}

export function checkStableMatchesLatest(stableManifestVersion, latestReleaseTag) {
  const latest = normalizeDesktopTag(latestReleaseTag)
  if (!stableManifestVersion || !latest) {
    return [
      `Could not read both values (stable manifest version='${stableManifestVersion ?? ''}', GitHub Latest tag='${latestReleaseTag ?? ''}').`,
    ]
  }
  if (stableManifestVersion !== latest) {
    return [
      `Stable updater manifest serves ${stableManifestVersion} but GitHub marks ${latestReleaseTag} as Latest (what beebeeb.io/download offers). Promote ${latest} to stable, or stop marking it Latest.`,
    ]
  }
  return []
}

async function fetchJson(url) {
  const headers = { accept: 'application/json', 'user-agent': 'beebeeb-desktop-release-check' }
  if (url.startsWith('https://api.github.com/') && process.env.GITHUB_TOKEN) {
    headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  }
  const res = await fetch(url, { headers })
  if (!res.ok) throw new Error(`GET ${url} -> HTTP ${res.status}`)
  return res.json()
}

async function main(argv) {
  const [command, ...rest] = argv

  if (command === 'notes') {
    const [version, channel, notesPath = 'RELEASE_NOTES.md'] = rest
    if (!version || !channel) {
      console.error('usage: release-channel-checks.mjs notes <version> <channel> [notes-path]')
      return 2
    }
    if (!fs.existsSync(notesPath)) {
      console.error(
        `::error file=${notesPath}::Missing ${notesPath} for Beebeeb Desktop ${version}. Author the release notes at the repo root, commit them, then rerun this workflow. See docs/RELEASING.md.`,
      )
      return 1
    }
    const errors = validateReleaseNotes(fs.readFileSync(notesPath, 'utf8'), version, channel)
    for (const error of errors) console.error(`::error file=${notesPath}::${error}`)
    if (errors.length === 0) console.log(`release notes OK: ${version} on ${channel}`)
    return errors.length === 0 ? 0 : 1
  }

  if (command === 'delivery') {
    const manifest = await fetchJson(STABLE_MANIFEST_URL)
    const latest = await fetchJson(LATEST_RELEASE_API)
    console.log(`stable manifest (${STABLE_MANIFEST_URL}): ${manifest.version}`)
    console.log(`GitHub Latest release: ${latest.tag_name}`)
    const errors = checkStableMatchesLatest(manifest.version, latest.tag_name)
    for (const error of errors) console.error(`::error::${error}`)
    if (errors.length === 0) console.log('delivery OK: stable manifest matches GitHub Latest')
    return errors.length === 0 ? 0 : 1
  }

  console.error('usage: release-channel-checks.mjs <notes|delivery> ...')
  return 2
}

if (process.argv[1] && fileURLToPath(import.meta.url) === fs.realpathSync(process.argv[1])) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      console.error(`::error::${error.message}`)
      process.exit(1)
    },
  )
}
