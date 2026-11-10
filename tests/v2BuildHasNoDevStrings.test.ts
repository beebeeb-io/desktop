// P1b gate: the dev-only strings (`__bbV2DevHost`, `__bbV2Seed`, `data-v2-state-result`) are in 0 built files, plus
// one positive control. The scanner is scripts/check-v2-dev-strings.mjs; the build check runs it on dist/.
import { describe, expect, test } from 'bun:test'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { DEV_STRINGS, scan } from '../scripts/check-v2-dev-strings.mjs'

const ROOT = resolve(import.meta.dir, '..')

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), 'bb-v2-dev-strings-test-'))
}

describe('the built app carries no dev-only strings (P1b)', () => {
  test('a planted dev string is counted once', () => {
    const dir = tempDir()
    try {
      writeFileSync(join(dir, 'app.js'), 'const k = "__bbV2Seed"\n')
      const result = scan(dir)
      expect(result.files).toBe(1)
      expect(result.devStrings).toBe(1)
      expect(result.hits).toEqual([{ file: join(dir, 'app.js'), string: '__bbV2Seed', count: 1 }])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  test('a clean dir counts 0', () => {
    const dir = tempDir()
    try {
      writeFileSync(join(dir, 'app.js'), 'const k = "hello"\n')
      const result = scan(dir)
      expect(result.files).toBe(1)
      expect(result.devStrings).toBe(0)
      expect(result.hits).toEqual([])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  test('the source tree is a positive control (the scanner is not scanning nothing)', () => {
    const result = scan(join(ROOT, 'src/v2'))
    expect(result.files).toBeGreaterThan(100)
    expect(result.devStrings).toBeGreaterThan(0)
    const found = new Set(result.hits.map((hit) => hit.string))
    expect(found.size).toBeGreaterThan(0)
    for (const string of found) expect(DEV_STRINGS).toContain(string)
  })
})
