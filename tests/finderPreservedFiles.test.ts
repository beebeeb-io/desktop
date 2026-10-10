/**
 * Task 1882 (P0): removing Beebeeb from Finder keeps the files that never reached the server
 * (spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md §5). This file pins the
 * kept-files note's pure decision and the one sentence, which must be the same string in
 * TypeScript (the Sync tab and the compact window) and in Rust (the app's alert after a
 * sign-out). The rendering is pinned in tests/macSettingsTabs.test.tsx ("Sync tab").
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { KEPT_FOLDER_ROW_SENTENCE, PRESERVED_FILES_SENTENCE, preservedFilesLine, preservedFilesNote } from '../src/macSettingsModel'

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
