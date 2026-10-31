import { describe, expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  checkStableMatchesLatest,
  validateReleaseNotes,
} from '../scripts/release-channel-checks.mjs'

const repoRoot = path.resolve(import.meta.dir, '..')

// The exact Install / Update paragraph that shipped in desktop-v0.8.3 while it was
// only ever published to the alpha manifest.
const V083_INSTALL_UPDATE =
  'Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.'

const CHANNEL_AWARE_INSTALL_UPDATE =
  'This release is published to the alpha channel first. Installs following alpha receive it through the in-app updater; beta and stable installs receive it only once it is promoted to their channel.'

// Task 2176: on macOS the in-app update cannot install (the App Sandbox denies replacing the bundle),
// so a Mac shows "Download update" and the notes say so.
const MAC_DOWNLOAD_LINE =
  'On macOS the app shows "Download update": download the `.dmg` and drag Beebeeb into Applications to replace the installed version.'

const STABLE_INSTALL_UPDATE =
  `Existing Windows installs receive this release through the in-app updater automatically. ${MAC_DOWNLOAD_LINE} For a fresh Windows install, download the NSIS \`setup.exe\` from the release assets below.`

const CHANNEL_AWARE_WINDOWS_INSTALL_UPDATE =
  `This release is published to the alpha channel first. Windows installs following alpha receive it through the in-app updater; beta and stable installs receive it only once it is promoted to their channel. ${MAC_DOWNLOAD_LINE}`

function notes(version: string, installUpdate: string) {
  return `# Beebeeb Desktop ${version} - test\n\nIntro.\n\n### Install / Update\n\n${installUpdate}\n`
}

describe('validateReleaseNotes', () => {
  test('rejects the 0.8.3 automatic-delivery claim on the alpha channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'alpha')
    expect(errors.filter((e) => e.includes('claims automatic delivery'))).toHaveLength(1)
    expect(errors.some((e) => e.includes('alpha channel only'))).toBe(true)
  })

  test('rejects the automatic-delivery claim on the beta channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'beta')
    expect(errors.some((e) => e.includes('beta channel only'))).toBe(true)
  })

  test('allows the Windows automatic-delivery claim with the macOS line when the release goes straight to stable', () => {
    expect(validateReleaseNotes(notes('0.8.21', STABLE_INSTALL_UPDATE), '0.8.21', 'stable')).toEqual([])
  })

  test('accepts the channel-aware wording on a non-stable channel', () => {
    expect(
      validateReleaseNotes(notes('0.9.0', CHANNEL_AWARE_WINDOWS_INSTALL_UPDATE), '0.9.0', 'alpha'),
    ).toEqual([])
  })

  test('still requires the exact version (0.8.3 notes do not satisfy 0.8.30)', () => {
    const errors = validateReleaseNotes(notes('0.8.3', CHANNEL_AWARE_WINDOWS_INSTALL_UPDATE), '0.8.30', 'alpha')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('exact release version 0.8.30')
  })

  test('rejects non-semver versions and unknown channels', () => {
    const errors = validateReleaseNotes(notes('0.9.0-beta', CHANNEL_AWARE_WINDOWS_INSTALL_UPDATE), '0.9.0-beta', 'nightly')
    expect(errors.some((e) => e.includes('plain semver'))).toBe(true)
    expect(errors.some((e) => e.includes("channel 'nightly'"))).toBe(true)
  })
})

describe('a Mac cannot install in-app (task 2176)', () => {
  test('rejects in-app delivery promised to every existing install, on stable too', () => {
    const errors = validateReleaseNotes(notes('0.8.21', V083_INSTALL_UPDATE), '0.8.21', 'stable')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('macOS')
    expect(errors[0]).toContain('Existing desktop installs receive this release through the in-app updater automatically')
  })

  test('rejects the unscoped channel-aware wording: Mac installs on that channel do not receive it in-app either', () => {
    const errors = validateReleaseNotes(notes('0.9.0', CHANNEL_AWARE_INSTALL_UPDATE), '0.9.0', 'alpha')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('without naming Windows')
    expect(errors[0]).toContain('Installs following alpha receive it through the in-app updater')
  })

  test('rejects Windows-scoped delivery that never says what a Mac does', () => {
    const windowsOnly = 'Existing Windows installs receive this release through the in-app updater automatically.'
    const errors = validateReleaseNotes(notes('0.8.21', windowsOnly), '0.8.21', 'stable')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('Download update')
  })

  test('notes that promise no in-app delivery need no macOS line', () => {
    expect(validateReleaseNotes(notes('0.8.21', 'Download the installer from the release assets.'), '0.8.21', 'stable')).toEqual([])
  })
})

describe('checkStableMatchesLatest', () => {
  test('flags the live 2026-09-25 drift: stable 0.8.2 vs GitHub Latest desktop-v0.8.3', () => {
    const errors = checkStableMatchesLatest('0.8.2', 'desktop-v0.8.3')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('serves 0.8.2')
    expect(errors[0]).toContain('desktop-v0.8.3')
  })

  test('passes when the stable manifest matches the Latest release', () => {
    expect(checkStableMatchesLatest('0.8.3', 'desktop-v0.8.3')).toEqual([])
  })

  test('fails closed when either value is missing', () => {
    expect(checkStableMatchesLatest('', 'desktop-v0.8.3')).toHaveLength(1)
    expect(checkStableMatchesLatest('0.8.3', undefined)).toHaveLength(1)
  })
})

describe('notes CLI (what the release workflow runs)', () => {
  function runNotesCli(version: string, channel: string, body: string) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'bb-release-notes-'))
    const file = path.join(dir, 'RELEASE_NOTES.md')
    fs.writeFileSync(file, body)
    try {
      execFileSync('node', ['scripts/release-channel-checks.mjs', 'notes', version, channel, file], {
        cwd: repoRoot,
        stdio: 'pipe',
      })
      return 0
    } catch (error) {
      return (error as { status: number }).status
    } finally {
      fs.rmSync(dir, { recursive: true, force: true })
    }
  }

  test('exits 1 on the 0.8.3 notes for alpha and for stable (a Mac cannot install in-app), 0 for the honest stable wording', () => {
    expect(runNotesCli('0.8.3', 'alpha', notes('0.8.3', V083_INSTALL_UPDATE))).toBe(1)
    expect(runNotesCli('0.8.3', 'stable', notes('0.8.3', V083_INSTALL_UPDATE))).toBe(1)
    expect(runNotesCli('0.8.21', 'stable', notes('0.8.21', STABLE_INSTALL_UPDATE))).toBe(0)
  })

  test('exits 1 when the notes file is missing', () => {
    let status = 0
    try {
      execFileSync('node', ['scripts/release-channel-checks.mjs', 'notes', '0.8.3', 'alpha', '/nonexistent/RELEASE_NOTES.md'], {
        cwd: repoRoot,
        stdio: 'pipe',
      })
    } catch (error) {
      status = (error as { status: number }).status
    }
    expect(status).toBe(1)
  })
})
