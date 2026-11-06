/**
 * Tests for src/windows/fileGlyph.tsx (LD-0105 U1 — review fixes).
 *
 * P2 guard: every var(--token) value in FILE_COLOR must reference a custom
 * property that is actually defined in design.css. This catches the original
 * bug where FILE_COLOR.default used `var(--ink3)` (no such token) instead of
 * `var(--ink-3)`.
 *
 * Mutation check methodology (AGENTS.md): the test was run against the unfixed
 * code (`var(--ink3)`) and failed: "Expected tokens to contain '--ink3'", then
 * passed once the value was corrected to `var(--ink-3)`.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { FILE_COLOR, getFileType, type FileType } from '../src/windows/fileGlyph'

const CSS_PATH = join(import.meta.dir, '..', 'src', 'design.css')

/** All custom-property names declared anywhere in design.css, e.g. `--ink-3`. */
function definedTokens(): Set<string> {
  const css = readFileSync(CSS_PATH, 'utf8')
  const names = new Set<string>()
  for (const m of css.matchAll(/--([a-zA-Z0-9-]+)\s*:/g)) {
    names.add('--' + m[1])
  }
  return names
}

describe('FILE_COLOR', () => {
  test('every var(--token) value references a custom property defined in design.css', () => {
    const tokens = definedTokens()
    expect(tokens.size).toBeGreaterThan(0) // guard: CSS was read

    const varEntries = Object.entries(FILE_COLOR).filter(([, v]) => v.startsWith('var('))
    expect(varEntries.length).toBeGreaterThan(0) // guard: at least one var() reference exists

    for (const [type, value] of varEntries) {
      const token = value.match(/^var\((--[a-zA-Z0-9-]+)\)$/)?.[1]
      expect(token, `FILE_COLOR.${type} = "${value}" is not a plain var(--token)`).toBeTruthy()
      expect(tokens, `FILE_COLOR.${type} uses "${value}" but ${token} is not in design.css`).toContain(token!)
    }
  })

  test('every FileType has an entry in FILE_COLOR', () => {
    const types: FileType[] = ['pdf', 'image', 'video', 'audio', 'code', 'archive', 'default']
    for (const t of types) {
      expect(FILE_COLOR[t], `FILE_COLOR.${t} is missing`).toBeTruthy()
    }
  })
})

describe('getFileType', () => {
  test('returns pdf for .pdf', () => expect(getFileType('report.pdf')).toBe('pdf'))
  test('returns image for .jpg', () => expect(getFileType('photo.jpg')).toBe('image'))
  test('returns video for .mp4', () => expect(getFileType('clip.mp4')).toBe('video'))
  test('returns audio for .mp3', () => expect(getFileType('song.mp3')).toBe('audio'))
  test('returns code for .ts', () => expect(getFileType('main.ts')).toBe('code'))
  test('returns archive for .zip', () => expect(getFileType('backup.zip')).toBe('archive'))
  test('returns default for .txt', () => expect(getFileType('notes.txt')).toBe('default'))
  test('returns default for a file with no extension', () => expect(getFileType('Makefile')).toBe('default'))
})
