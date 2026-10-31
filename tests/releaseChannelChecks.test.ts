import { describe, expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  checkStableMatchesLatest,
  MAC_INSTALL_LINE,
  validateReleaseNotes,
} from '../scripts/release-channel-checks.mjs'

const repoRoot = path.resolve(import.meta.dir, '..')

// The exact Install / Update paragraph that shipped in desktop-v0.8.3 while it was
// only ever published to the alpha manifest (and in 0.8.13 to 0.8.20, false for every Mac).
const V083_INSTALL_UPDATE =
  'Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.'

const CHANNEL_AWARE_INSTALL_UPDATE =
  'This release is published to the alpha channel first. Installs following alpha receive it through the in-app updater; beta and stable installs receive it only once it is promoted to their channel.'

const FRESH_INSTALL =
  'For a fresh Windows install, download the NSIS `setup.exe` from the release assets below; for macOS, the `.dmg` from the same release.'

// The docs/RELEASING.md template (task 2176): delivery scoped to Windows, plus the one macOS line.
const STABLE_INSTALL_UPDATE = `Existing Windows installs receive this release through the in-app updater automatically.\n\n${MAC_INSTALL_LINE}\n\n${FRESH_INSTALL}`

const CHANNEL_AWARE_WINDOWS_INSTALL_UPDATE = `This release is published to the alpha channel first. Windows installs following alpha receive it through the in-app updater; beta and stable installs receive it only once it is promoted to their channel.\n\n${MAC_INSTALL_LINE}\n\n${FRESH_INSTALL}`

function notes(version: string, installUpdate: string) {
  return `# Beebeeb Desktop ${version} - test\n\nIntro.\n\n### Install / Update\n\n${installUpdate}\n`
}

const scoped = (errors: string[]) => errors.filter((e) => e.includes('without scoping it to Windows'))
const macLineMissing = (errors: string[]) => errors.filter((e) => e.includes('must contain the macOS line'))

describe('validateReleaseNotes', () => {
  test('rejects the 0.8.3 automatic-delivery claim on the alpha channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'alpha')
    const channel = errors.filter((e) => e.includes('claims automatic delivery'))
    expect(channel).toHaveLength(1)
    expect(channel[0]).toContain('alpha channel only')
  })

  test('rejects the automatic-delivery claim on the beta channel', () => {
    const errors = validateReleaseNotes(notes('0.8.3', V083_INSTALL_UPDATE), '0.8.3', 'beta')
    const channel = errors.filter((e) => e.includes('claims automatic delivery'))
    expect(channel).toHaveLength(1)
    expect(channel[0]).toContain('beta channel only')
  })

  test('accepts the stable template: Windows delivery and the macOS line', () => {
    expect(validateReleaseNotes(notes('0.8.21', STABLE_INSTALL_UPDATE), '0.8.21', 'stable')).toEqual([])
  })

  test('accepts the channel-aware template on a non-stable channel', () => {
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
  test('the 0.8.13-0.8.20 wording fails on stable: unscoped delivery and no macOS line', () => {
    const errors = validateReleaseNotes(notes('0.8.21', V083_INSTALL_UPDATE), '0.8.21', 'stable')
    expect(errors).toHaveLength(2)
    expect(scoped(errors)).toHaveLength(1)
    expect(scoped(errors)[0]).toContain('Existing desktop installs receive this release through the in-app updater automatically.')
    expect(macLineMissing(errors)).toHaveLength(1)
  })

  test('the unscoped channel-aware wording fails: Mac installs on that channel do not receive it in-app either', () => {
    const errors = validateReleaseNotes(notes('0.9.0', `${CHANNEL_AWARE_INSTALL_UPDATE}\n\n${MAC_INSTALL_LINE}`), '0.9.0', 'alpha')
    expect(errors).toHaveLength(1)
    expect(scoped(errors)[0]).toContain('Installs following alpha receive it through the in-app updater')
  })

  test('Windows-scoped delivery with only a fresh-install .dmg line still lacks the macOS line', () => {
    const errors = validateReleaseNotes(
      notes('0.8.21', `Existing Windows installs receive this release through the in-app updater automatically.\n\n${FRESH_INSTALL}`),
      '0.8.21',
      'stable',
    )
    expect(errors).toHaveLength(1)
    expect(macLineMissing(errors)).toHaveLength(1)
  })

  test('the macOS line is required even when nothing promises in-app delivery', () => {
    const errors = validateReleaseNotes(notes('0.8.21', FRESH_INSTALL), '0.8.21', 'stable')
    expect(errors).toHaveLength(1)
    expect(macLineMissing(errors)).toHaveLength(1)
  })

  test('notes without an Install / Update section fail', () => {
    const errors = validateReleaseNotes('# Beebeeb Desktop 0.8.21\n\nIntro.\n', '0.8.21', 'stable')
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('no "### Install / Update" section')
  })

  test('reworded or misspelt delivery promises are caught on stable', () => {
    for (const promise of [
      'Existing desktop installs update automatically.',
      'Existing desktop installs get it as an in-app update automatically.',
      'Existing desktop installs receive it through the updater automatically.',
      'Existing desktop installs receive it through the auto-updater.',
      'Existing desktop installs receive it through the in app updater.',
      'Existing desktop installs receive it through the in‑app updater.',
      'Existing desktop installs receive 0.8.21 through the in-app updater.',
      'Existing Windows and macOS installs receive this release through the in-app updater automatically.',
    ]) {
      const errors = validateReleaseNotes(notes('0.8.21', `${promise}\n\n${MAC_INSTALL_LINE}`), '0.8.21', 'stable')
      expect({ promise, scoped: scoped(errors).length }).toEqual({ promise, scoped: 1 })
    }
  })

  test('a hard-wrapped sentence is read as one sentence, both ways', () => {
    const wrappedWindows = 'Existing Windows installs receive this release through the\nin-app updater automatically.'
    expect(validateReleaseNotes(notes('0.8.21', `${wrappedWindows}\n\n${MAC_INSTALL_LINE}`), '0.8.21', 'stable')).toEqual([])
    const wrappedDesktop = 'Existing desktop installs receive this release through\nthe in-app updater.'
    expect(scoped(validateReleaseNotes(notes('0.8.21', `${wrappedDesktop}\n\n${MAC_INSTALL_LINE}`), '0.8.21', 'stable'))).toHaveLength(1)
    const wrappedMacLine = MAC_INSTALL_LINE.replace(' and drag ', '\nand drag ')
    expect(validateReleaseNotes(notes('0.8.21', wrappedMacLine), '0.8.21', 'stable')).toEqual([])
  })

  test('lowercase "windows" scopes a sentence; "i.e." and version numbers do not split one', () => {
    for (const promise of [
      'Existing windows installs receive this release through the in-app updater automatically.',
      'Windows installs, i.e. existing ones, receive 0.8.21 through the in-app updater automatically.',
    ]) {
      expect(validateReleaseNotes(notes('0.8.21', `${promise}\n\n${MAC_INSTALL_LINE}`), '0.8.21', 'stable')).toEqual([])
    }
  })

  test('the docs/RELEASING.md template carries the macOS line word for word', () => {
    const releasing = fs.readFileSync(path.join(repoRoot, 'docs', 'RELEASING.md'), 'utf8')
    expect(releasing.replace(/\s+/g, ' ')).toContain(MAC_INSTALL_LINE)
  })

  test('only the Install / Update section is held to the scope rule', () => {
    const body = `# Beebeeb Desktop 0.8.21\n\n- **macOS:** sync resumes automatically after sleep.\n\n### Install / Update\n\n${MAC_INSTALL_LINE}\n`
    expect(validateReleaseNotes(body, '0.8.21', 'stable')).toEqual([])
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
