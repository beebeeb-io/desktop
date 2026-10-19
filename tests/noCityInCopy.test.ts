import { describe, expect, test } from 'bun:test'
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { hits, scan, stripRustTestItems } from './fixtures/cityGuard'

/**
 * Task 2050 (Guus, 2026-10-19): no city in the desktop. See tests/fixtures/cityGuard.ts for the two
 * rules (a city name, and a read of a `city` field) and what is scanned. `GUARD_ROOT` points the
 * scan at another checkout (used to prove the guard is red on the code before task 2050).
 */
const ROOT = process.env.GUARD_ROOT ?? fileURLToPath(new URL('..', import.meta.url))

describe('the guard itself', () => {
  test('flags a city name in a string, a template and JSX text, in TS and in Rust', () => {
    expect(hits(`const a = 'Stored in Falkenstein'`, 'ts')).toHaveLength(1)
    expect(hits('const b = `Uploading to ${x} Helsinki`', 'ts')).toHaveLength(1)
    expect(hits('<p>Nürnberg, Germany</p>', 'ts')).toHaveLength(1)
    expect(hits(`const region = 'Ede'`, 'ts')).toHaveLength(1)
    expect(hits('let s = "Nuremberg";', 'rs')).toHaveLength(1)
  })

  test('flags a read or declaration of a city field', () => {
    expect(hits('return choice.city ?? choice.name', 'ts')).toHaveLength(1)
    expect(hits('  city: string | null', 'ts')).toHaveLength(1)
    expect(hits("const { city } = row", 'ts')).toHaveLength(1)
    expect(hits('  pub city: Option<String>,', 'rs')).toHaveLength(1)
    expect(hits('  #[serde(alias = "example_city")]', 'rs')).toHaveLength(1)
  })

  test('ignores comments and words that merely contain the letters', () => {
    expect(hits('// Falkenstein is a comment', 'ts')).toHaveLength(0)
    expect(hits('/* Helsinki */ const a = 1', 'ts')).toHaveLength(0)
    expect(hits('/// no `city` and no example_city here', 'rs')).toHaveLength(0)
    expect(hits('const needed = precede + capacity + velocity', 'ts')).toHaveLength(0)
    expect(hits(`const u = 'https://example.com/a'`, 'ts')).toHaveLength(0)
  })

  test('Rust test items may name a city to prove it is absent; the code around them may not', () => {
    const src = [
      'fn shipped() -> &\'static str { "Helsinki" }',
      '#[cfg(test)]',
      'mod tests {',
      '    fn t() { assert_eq!(x, "Falkenstein {"); }',
      '}',
      'fn after() -> &\'static str { "Nuremberg" }',
    ].join('\n')
    expect(stripRustTestItems(src)).not.toContain('Falkenstein')
    expect(hits(src, 'rs').map(h => h.split(':')[0])).toEqual(['1', '6'])
  })

  test('a mutated copy of the real tree goes red (the scan finds an injected city, in TS and in Rust)', () => {
    const scratch = mkdtempSync(join(tmpdir(), 'city-guard-'))
    try {
      cpSync(join(ROOT, 'src'), join(scratch, 'src'), { recursive: true })
      cpSync(join(ROOT, 'src-tauri/src'), join(scratch, 'src-tauri/src'), { recursive: true })
      const total = (o: Record<string, string[]>) => Object.values(o).reduce((n, h) => n + h.length, 0)
      const before = total(scan(scratch).offenders)
      const tsFile = join(scratch, 'src/windows/useRegion.ts')
      writeFileSync(tsFile, `${readFileSync(tsFile, 'utf8')}\nexport const leaked = 'Stored in Helsinki'\n`)
      const rsFile = join(scratch, 'src-tauri/src/account_dto.rs')
      writeFileSync(rsFile, `${readFileSync(rsFile, 'utf8')}\npub fn leaked() -> &'static str { "Falkenstein" }\n`)
      const after = scan(scratch).offenders
      expect(total(after)).toBe(before + 2)
      expect(after['src/windows/useRegion.ts'].at(-1)).toContain('city name')
      expect(after['src-tauri/src/account_dto.rs'].at(-1)).toContain('city name')
    } finally {
      rmSync(scratch, { recursive: true, force: true })
    }
  })
})

describe('no city in the desktop', () => {
  const result = scan(ROOT)

  test('the scan found the source trees (a guard that scans nothing is red)', () => {
    expect(result.files).toBeGreaterThan(100)
  })

  test('no city name and no read of a city field in src/ or src-tauri/src/ (outside comments and cfg(test) items)', () => {
    expect(result.offenders).toEqual({})
  })
})
