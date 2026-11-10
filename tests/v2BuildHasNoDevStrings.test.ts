// P1b (desktop v2): the dev-only strings are in zero built files. The scanner in scripts/check-v2-dev-strings.mjs walks a
// build output; this test proves it counts a planted file (so a "0" is not a scan of nothing), counts a clean file as 0,
// and that the source tree is a positive control (it holds the strings the build must lose). The build itself is checked
// by the CLI: `bun run build && node scripts/check-v2-dev-strings.mjs dist`.
import { describe, expect, test } from 'bun:test'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { scan, DEV_STRINGS } from '../scripts/check-v2-dev-strings.mjs'

const ROOT = resolve(import.meta.dir, '..')

function tempDir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `bb-v2-dev-strings-${prefix}-`))
}

describe('the dev-string scanner counts what ships (P1b)', () => {
  test('a planted build file holding __bbV2Seed is counted once', () => {
    const dir = tempDir('planted')
    try {
      mkdirSync(join(dir, 'assets'))
      writeFileSync(join(dir, 'assets', 'app.js'), "globalThis['__bbV2Seed'] = {}\n")
      const result = scan(dir)
      expect(result.files).toBe(1)
      expect(result.devStrings).toBe(1)
      expect(result.hits).toEqual([{ file: join('assets', 'app.js'), string: '__bbV2Seed', count: 1 }])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  test('a clean build directory counts 0', () => {
    const dir = tempDir('clean')
    try {
      writeFileSync(join(dir, 'app.js'), 'export const answer = 42\n')
      const result = scan(dir)
      expect(result.files).toBe(1)
      expect(result.devStrings).toBe(0)
      expect(result.hits).toEqual([])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  test('the source tree is a positive control: it holds at least one of the three strings', () => {
    const result = scan(join(ROOT, 'src/v2'), /\.(ts|tsx)$/)
    expect(result.files).toBeGreaterThan(100)
    expect(result.devStrings).toBeGreaterThan(0)
    const found = new Set(result.hits.map((h: { string: string }) => h.string))
    expect(DEV_STRINGS.some((s) => found.has(s))).toBe(true)
  })
})
