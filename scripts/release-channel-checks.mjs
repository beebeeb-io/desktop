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
// 3. Task 2176: on macOS the in-app update cannot install (the App Sandbox denies
//    replacing the bundle), yet desktop-v0.8.13 to v0.8.20 told every existing install
//    it would update in-app. `notes` requires MAC_INSTALL_LINE in the Install / Update
//    section on every channel, and refuses a delivery sentence there that is not
//    scoped to Windows.
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

// Task 2176. The one macOS sentence every release's Install / Update section carries. It is
// true on every channel and for every installed build: builds up to 0.8.20 offer "Restart to
// update", which fails under the sandbox, and later builds offer a download (stable) or the
// release page (alpha/beta) instead (src/macUpdateOffer.ts). Every release ships a .dmg.
export const MAC_INSTALL_LINE =
  'On macOS the in-app update cannot install: download the `.dmg` from this release and drag Beebeeb into Applications to replace the installed version.'

// When macOS is not in this release (macOS items in RELEASE_BLOCKERS.md bind at the
// release ref), the Install / Update section must carry this line instead of MAC_INSTALL_LINE
// to tell Mac users what is happening. The macOS build follows in a later version; it
// does NOT get a re-cut of this one.
export const MACOS_NOT_IN_RELEASE_LINE =
  'macOS: not in this release; it follows in a later version.'

// A sentence about delivering the release to existing installs (hyphen variants included).
const DELIVERY_WORDS = /\b(?:in[-\u2010\u2011 ]?app|(?:auto[-\u2010\u2011 ]?)?updat(?:e|er|es|ed|ing)|automatically)\b/i

// A line of the notes that the lead still has to fill before the release: `<!-- lead: ... -->`. HTML
// comments are invisible on the release page, so a note left like this ships an evidence line
// that says nothing ("- Test suite:"). `notes` fails while one remains.
const LEAD_MARKER = /<!--\s*lead\s*:[\s\S]*?(?:-->|(?![\s\S]))/gi

// The open lead markers of the notes, each as "line N: <its first words>".
export function openLeadMarkers(notes) {
  return [...notes.matchAll(LEAD_MARKER)].map((match) => {
    const line = notes.slice(0, match.index).split('\n').length
    const text = collapse(match[0].replace(/^<!--\s*/, '').replace(/\s*-->$/, ''))
    return `line ${line}: ${text.length > 80 ? `${text.slice(0, 80)}...` : text}`
  })
}

function collapse(text) {
  return text.replace(/\s+/g, ' ').trim()
}

// The text of the `### Install / Update` section, without HTML comments, or null.
function installUpdateSection(notes) {
  const match = notes.match(/^#{2,4}\s*Install\s*\/\s*Update\s*$([\s\S]*?)(?=^#{1,4}\s|(?![\s\S]))/im)
  return match ? match[1].replace(/<!--[\s\S]*?-->/g, '') : null
}

// Sentences of a section: paragraphs and list items are joined across hard wraps, then split
// after . ! ? followed by a capital (so version numbers and "i.e." do not split).
function sentences(section) {
  return section
    .split(/\n\s*\n|\n\s*[-*]\s+/)
    .map(collapse)
    .filter(Boolean)
    .flatMap((paragraph) => paragraph.split(/(?<=[.!?])\s+(?=[A-Z"`*(])/))
    .map((s) => s.trim())
    .filter(Boolean)
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Validate RELEASE_NOTES.md content.
 *
 * @param {string} notes - File content.
 * @param {string} version - Semver like "0.8.21".
 * @param {string} channel - "alpha", "beta", or "stable".
 * @param {{ noMacos?: boolean }} [options]
 *   noMacos: true when macOS items bind at the release ref (Windows+Linux-only release).
 *     Skips the MAC_INSTALL_LINE check and instead requires MACOS_NOT_IN_RELEASE_LINE.
 * @returns {string[]} List of error messages; empty on success.
 */
export function validateReleaseNotes(notes, version, channel, options = {}) {
  const { noMacos = false } = options
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

  const markers = openLeadMarkers(notes)
  if (markers.length > 0) {
    errors.push(
      `RELEASE_NOTES.md still holds ${markers.length} line(s) the lead has to fill before a release ("<!-- lead: ... -->"): ${markers.join('; ')}. Replace each with what was measured, or remove the item it belongs to.`,
    )
  }

  const section = installUpdateSection(notes)
  if (section === null) {
    errors.push(
      'RELEASE_NOTES.md has no "### Install / Update" section. Use the template in docs/RELEASING.md (task 2176: it carries the macOS line).',
    )
  } else if (noMacos) {
    // Windows+Linux-only release: macOS is not in this version.
    // Require the "not in this release" statement; MAC_INSTALL_LINE is not expected.
    const notInReleaseLine = collapse(MACOS_NOT_IN_RELEASE_LINE)
    if (!collapse(section).includes(notInReleaseLine)) {
      errors.push(
        `RELEASE_NOTES.md's Install / Update section must state that macOS is not in this release, word for word: "${MACOS_NOT_IN_RELEASE_LINE}" macOS items in RELEASE_BLOCKERS.md bind at this ref; macOS follows in a later version. See docs/RELEASING.md.`,
      )
    }
  } else {
    const macLine = collapse(MAC_INSTALL_LINE)
    if (!collapse(section).includes(macLine)) {
      errors.push(
        `RELEASE_NOTES.md's Install / Update section must contain the macOS line, word for word: "${MAC_INSTALL_LINE}" On macOS the in-app update cannot install (task 2176).`,
      )
    }
    for (const sentence of sentences(section)) {
      if (macLine.includes(sentence) || !DELIVERY_WORDS.test(sentence)) continue
      if (!/\bwindows\b/i.test(sentence) || /\bmac(?:os)?\b/i.test(sentence)) {
        errors.push(
          `RELEASE_NOTES.md's Install / Update section says how existing installs get the release without scoping it to Windows ("${sentence}"). On macOS the in-app update cannot install (task 2176): name Windows only, and say what a Mac does with the macOS line from docs/RELEASING.md.`,
        )
      }
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
    // Parse flags: --no-macos (Windows+Linux-only release; macOS follows in a later version).
    const flags = rest.filter((a) => a.startsWith('--'))
    const positional = rest.filter((a) => !a.startsWith('--'))
    const noMacos = flags.includes('--no-macos')
    const unknownFlags = flags.filter((f) => f !== '--no-macos')
    if (unknownFlags.length > 0) {
      console.error(`unknown flag(s): ${unknownFlags.join(' ')}`)
      console.error('usage: release-channel-checks.mjs notes <version> <channel> [notes-path] [--no-macos]')
      return 2
    }

    const [version, channel, notesPath = 'RELEASE_NOTES.md'] = positional
    if (!version || !channel) {
      console.error('usage: release-channel-checks.mjs notes <version> <channel> [notes-path] [--no-macos]')
      return 2
    }
    if (!fs.existsSync(notesPath)) {
      console.error(
        `::error file=${notesPath}::Missing ${notesPath} for Beebeeb Desktop ${version}. Author the release notes at the repo root, commit them, then rerun this workflow. See docs/RELEASING.md.`,
      )
      return 1
    }
    const errors = validateReleaseNotes(fs.readFileSync(notesPath, 'utf8'), version, channel, { noMacos })
    for (const error of errors) console.error(`::error file=${notesPath}::${error}`)
    if (errors.length === 0) console.log(`release notes OK: ${version} on ${channel}${noMacos ? ' (windows+linux only, macOS follows)' : ''}`)
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
