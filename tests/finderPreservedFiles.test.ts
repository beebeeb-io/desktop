/**
 * Task 1882 (P0): removing Beebeeb from Finder keeps the files that never reached the server
 * (spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md §5). This file pins the
 * kept-files note's pure decision and the one sentence, which must be the same string in
 * TypeScript (the Sync tab and the compact window) and in Rust (the app's alert after a
 * sign-out). The rendering is pinned in tests/macSettingsTabs.test.tsx ("Sync tab").
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import {
  KEPT_FOLDER_ROW_SENTENCE,
  keptFolderAfterDismiss,
  PRESERVED_FILES_SENTENCE,
  preservedFilesLine,
  preservedFilesNote,
} from '../src/macSettingsModel'

const FOLDER = '/Users/sam/Library/CloudStorage/Beebeeb (kept) /Notes '

describe('kept-files note (1882)', () => {
  test('a reported folder gives the sentence and that exact folder, byte for byte', () => {
    expect(preservedFilesNote({ preserved_location: FOLDER })).toEqual({ sentence: PRESERVED_FILES_SENTENCE, path: FOLDER })
  })

  test('no folder reported gives no note: null, absent, or blank', () => {
    expect(preservedFilesNote({ preserved_location: null })).toBeNull()
    expect(preservedFilesNote({})).toBeNull()
    expect(preservedFilesNote({ preserved_location: '   ' })).toBeNull()
  })

  test('1882 r5: after a Dismiss the row is gone only if Rust cleared it, else it shows the folder saved now', () => {
    const NEWER = '/Users/sam/Library/CloudStorage/Beebeeb (kept 2)'
    expect(keptFolderAfterDismiss({ cleared: true, current: null })).toBeNull()
    // A stale dismiss: nothing cleared, the newer folder is what the row shows.
    expect(keptFolderAfterDismiss({ cleared: false, current: NEWER })).toBe(NEWER)
    // Nothing is saved any more (dismissed elsewhere): no row.
    expect(keptFolderAfterDismiss({ cleared: false, current: null })).toBeNull()
    expect(keptFolderAfterDismiss({ cleared: false, current: '  ' })).toBeNull()
    // `cleared` wins: a row Rust cleared never reappears from a stray `current`.
    expect(keptFolderAfterDismiss({ cleared: true, current: NEWER })).toBeNull()
  })

  test('1882 r5: the dismiss result has the two keys Rust sends, in its struct', () => {
    const rust = readFileSync(join(import.meta.dir, '..', 'src-tauri', 'src', 'finder_removal.rs'), 'utf8').replace(/\r\n/g, '\n')
    const match = rust.match(/pub struct DismissOutcome \{\s*pub cleared: bool,\s*pub current: Option<String>,\s*\}/)
    expect(match).not.toBeNull()
    const lib = readFileSync(join(import.meta.dir, '..', 'src-tauri', 'src', 'lib.rs'), 'utf8').replace(/\r\n/g, '\n')
    expect(lib).toContain('fn dismiss_kept_unsynced_folder(path: String) -> Result<finder_removal::DismissOutcome, String>')
  })

  test('the compact window gets one line: the sentence, then the folder', () => {
    expect(preservedFilesLine({ preserved_location: FOLDER })).toBe(`${PRESERVED_FILES_SENTENCE} ${FOLDER}`)
    expect(preservedFilesLine({ preserved_location: null })).toBeNull()
  })

  test('the sentence is the spec\'s, names no provider, and is the same string Rust shows in its alert', () => {
    expect(PRESERVED_FILES_SENTENCE).toBe('Files that hadn’t reached your vault yet were kept on this Mac, in this folder:')
    for (const word of ['apple', 'icloud', 'hetzner', 'file provider', 'fileprovider', 'cloudstorage', '/']) {
      expect(PRESERVED_FILES_SENTENCE.toLowerCase()).not.toContain(word)
    }
    const rust = readFileSync(join(import.meta.dir, '..', 'src-tauri', 'src', 'finder_removal.rs'), 'utf8')
    const match = rust.match(/pub const PRESERVED_FILES_SENTENCE: &str =\s*"([^"]*)";/)
    expect(match?.[1]).toBe(PRESERVED_FILES_SENTENCE)
  })

  test('round 3 (re-review D3): the saved row\'s sentence is neutral and names no provider', () => {
    // The row outlives the sign-out and can be read by another account, so it must not claim the
    // files were meant for "your vault". The alert and the Repair line, which appear straight
    // after the removal, keep the sentence above.
    expect(KEPT_FOLDER_ROW_SENTENCE).toBe('Files that had not reached the server were kept in this folder:')
    expect(KEPT_FOLDER_ROW_SENTENCE).not.toBe(PRESERVED_FILES_SENTENCE)
    expect(KEPT_FOLDER_ROW_SENTENCE.toLowerCase()).not.toMatch(/\byour\b|vault/)
    for (const word of ['apple', 'icloud', 'hetzner', 'file provider', 'fileprovider', 'cloudstorage', '/']) {
      expect(KEPT_FOLDER_ROW_SENTENCE.toLowerCase()).not.toContain(word)
    }
  })

  test('round 2 (review I2): the kept folder\'s path wraps and is never cut off', () => {
    const css = readFileSync(join(import.meta.dir, '..', 'src', 'macSettings.css'), 'utf8')
    const rule = css.match(/\.ms-mono--wrap\s*\{([^}]*)\}/)
    expect(rule).not.toBeNull()
    const body = rule![1].replace(/\s+/g, ' ')
    expect(body).toContain('white-space: normal;')
    expect(body).toContain('overflow-wrap: anywhere;')
    expect(body).toContain('text-overflow: clip;')
    expect(body).toContain('overflow: visible;')
  })
})

// Rebase re-review Minor 4 (lead ruling, 2026-10-10): 1882 r4's "Repair failed after the removal" note can occur on
// no platform under spec A. A Mac saves nothing after its removal, and Windows and Linux have no File Provider domain,
// so their removal never works. The note, its code and its copy are gone. The copy a failed Repair still shows is
// pinned where it is shown: tests/macSettingsTabs.test.tsx and tests/finderInstallOneSurface.test.tsx.
describe('no Repair-failed-after-the-removal note (rebase re-review Minor 4)', () => {
  const sources = (dir: string): string[] =>
    readdirSync(dir, { recursive: true, encoding: 'utf8' })
      .filter((name) => /\.(ts|tsx)$/.test(name))
      .map((name) => join(dir, name))

  test('no frontend file names the r4 code, its helpers or its copy', () => {
    const files = sources(join(import.meta.dir, '..', 'src'))
    expect(files.length).toBeGreaterThan(20) // the walk read the frontend
    for (const file of files) {
      const text = readFileSync(file, 'utf8')
      for (const r4 of ['repair_failed_after_removal', 'REPAIR_FAILED_AFTER_REMOVAL_CODE', 'repairRemoved', 'REPAIR_REMOVED', 'Beebeeb was removed from Finder', 'is no longer in Finder']) {
        expect({ file, r4, named: text.includes(r4) }).toEqual({ file, r4, named: false })
      }
    }
  })

  test('Rust no longer builds the code', () => {
    const rust = readFileSync(join(import.meta.dir, '..', 'src-tauri', 'src', 'finder_removal.rs'), 'utf8')
    expect(rust).toContain('pub const PRESERVED_FILES_SENTENCE') // the file read is the right one
    expect(rust).not.toContain('REPAIR_FAILED_AFTER_REMOVAL_CODE')
    expect(rust).not.toContain('fn repair_save_error')
  })
})
