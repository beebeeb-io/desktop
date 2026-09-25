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

function notes(version: string, installUpdate: string) {
  return `# Beebeeb Desktop ${version} - test\n\nIntro.\n\n### Install / Update\n\n${installUpdate}\n`
}

describe('validateReleaseNotes', () => {
  test('rejects the 0.8.3 automatic-delivery claim on the alpha channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'alpha')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('claims automatic delivery')
    expect(errors[0]).toContain('alpha channel only')
  })

  test('rejects the automatic-delivery claim on the beta channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'beta')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('beta channel only')
  })

  test('allows the automatic-delivery claim when the release goes straight to stable', () => {
    expect(validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'stable')).toEqual([])
  })

  test('accepts the channel-aware wording on a non-stable channel', () => {
    expect(
      validateReleaseNotes(notes('0.9.0', CHANNEL_AWARE_INSTALL_UPDATE), '0.9.0', 'alpha'),
    ).toEqual([])
  })

  test('still requires the exact version (0.8.3 notes do not satisfy 0.8.30)', () => {
    const errors = validateReleaseNotes(notes('0.8.3', CHANNEL_AWARE_INSTALL_UPDATE), '0.8.30', 'alpha')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('exact release version 0.8.30')
  })

  test('rejects non-semver versions and unknown channels', () => {
    const errors = validateReleaseNotes(notes('0.9.0-beta', CHANNEL_AWARE_INSTALL_UPDATE), '0.9.0-beta', 'nightly')
    expect(errors.some((e) => e.includes('plain semver'))).toBe(true)
    expect(errors.some((e) => e.includes("channel 'nightly'"))).toBe(true)
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

  test('exits 1 on the 0.8.3 notes for alpha and 0 for stable', () => {
    expect(runNotesCli('0.8.3', 'alpha', notes('0.8.3', V083_INSTALL_UPDATE))).toBe(1)
    expect(runNotesCli('0.8.3', 'stable', notes('0.8.3', V083_INSTALL_UPDATE))).toBe(0)
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
