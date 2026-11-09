// P1c (desktop v2): no production module draws the board's example data. A production module is every .ts/.tsx under
// src/v2 except the board's own drawing kit: the fixtures (source/fixtures/, source/fixtures.ts, tray/fixtures.ts), the
// frame tables (frames/, the `?frame=` drawings, loaded only on the dev server) and the dev server's live state route
// (dev/). None of them may import from that kit: not a value, not a type, not a re-export, not a dynamic import.
// The one door is live.tsx's `import('./dev/live')`, which runs only while `import.meta.env.DEV` is true (a production
// build drops it; the build check in the P1c PR counts 0 fixture strings in dist/).
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, posix, relative, resolve } from 'node:path'

const ROOT = resolve(import.meta.dir, '..')
const V2 = join(ROOT, 'src/v2')

/** The board's drawing kit: production code never reaches into it. Paths are repo-relative, without an extension. */
const KIT = ['src/v2/dev/', 'src/v2/frames/', 'src/v2/source/fixtures/', 'src/v2/source/fixtures', 'src/v2/tray/fixtures']

/** Every allowed reach into the kit, with its reason. */
const DOORS: Record<string, { to: string; why: string }> = {
  'src/v2/live.tsx': { to: 'src/v2/dev/live', why: 'the live state route, behind `if (!import.meta.env.DEV) return Window`' },
}

/** The board's example people and files (source/fixtures/*), and the FIXTURE_ naming the example data carries. */
const EXAMPLE_WORDS = [/\bAnna\b/, /Lotte de Wit/, /Priya Shah/, /Harbourlight/, /Render final/, /Wedding 2025/, /\bconst FIXTURE_[A-Z_]+\s*[:=]/]

const inKit = (rel: string) => KIT.some((k) => (k.endsWith('/') ? rel.startsWith(k) : rel === k || rel.startsWith(`${k}/`) || rel === `${k}.ts` || rel === `${k}.tsx`))

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (/\.tsx?$/.test(name)) out.push(p)
  }
  return out
}

/** Block comments and whole-line comments out, so a comment that names a fixture is not an import. */
const code = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, '').split('\n').filter((l) => !/^\s*\/\//.test(l)).join('\n')

/** Every module specifier in the source: `import … from`, `export … from`, `import type`, `import 'x'` and `import('x')`. */
export function specifiers(src: string): string[] {
  const body = code(src)
  const found: string[] = []
  for (const re of [/\bfrom\s*['"]([^'"]+)['"]/g, /\bimport\s*\(\s*['"]([^'"]+)['"]\s*\)/g, /\bimport\s+['"]([^'"]+)['"]/g]) {
    for (const m of body.matchAll(re)) found.push(m[1])
  }
  return found
}

/** The kit modules a production file reaches, repo-relative, without the doors. */
export function reaches(file: string, src: string): string[] {
  const dir = posix.dirname(file)
  return specifiers(src)
    .filter((s) => s.startsWith('.'))
    .map((s) => posix.normalize(posix.join(dir, s)).replace(/\.(tsx?|js)$/, ''))
    .filter((target) => inKit(target) && DOORS[file]?.to !== target)
}

const production = walk(V2).map((p) => relative(ROOT, p).split('\\').join('/')).filter((rel) => !inKit(rel.replace(/\.tsx?$/, '')) && !inKit(rel))

describe('no production module in src/v2 imports the board’s fixtures or the dev server’s code (P1c)', () => {
  test('the scan reads the production modules (a scan of nothing would pass)', () => {
    expect(production.length).toBeGreaterThan(120)
    expect(production).toContain('src/v2/V2App.tsx')
    expect(production).toContain('src/v2/sections/settings/backups/index.tsx')
    expect(production.some((p) => p.startsWith('src/v2/frames/') || p.startsWith('src/v2/dev/') || p.includes('/fixtures'))).toBe(false)
  })

  test('the checker sees every import form, and only real imports (planted cases)', () => {
    const f = 'src/v2/sections/x/index.tsx'
    expect(reaches(f, "import { A } from '../../source/fixtures/board'")).toEqual(['src/v2/source/fixtures/board'])
    expect(reaches(f, "import type { T } from '../../source/fixtures/photos'")).toEqual(['src/v2/source/fixtures/photos'])
    expect(reaches(f, "export { NOW } from '../../tray/fixtures'")).toEqual(['src/v2/tray/fixtures'])
    expect(reaches(f, "import {\n  a,\n  b,\n} from '../../source/fixtures'")).toEqual(['src/v2/source/fixtures'])
    expect(reaches(f, "void import('../../dev/live')")).toEqual(['src/v2/dev/live'])
    expect(reaches(f, "import { FRAMES } from '../../frames/registry'")).toEqual(['src/v2/frames/registry'])
    expect(reaches(f, "// import { A } from '../../source/fixtures/board'\n/* from '../../dev/x' */")).toEqual([])
    expect(reaches(f, "import { pickSource } from '../../source'")).toEqual([])
    expect(reaches('src/v2/live.tsx', "void import('./dev/live')")).toEqual([])
    expect(reaches('src/v2/live.tsx', "import { x } from './dev/reads'")).toEqual(['src/v2/dev/reads'])
  })

  test('0 production modules reach into the kit', () => {
    const bad: string[] = []
    for (const rel of production) {
      for (const target of reaches(rel, readFileSync(join(ROOT, rel), 'utf8'))) bad.push(`${rel} -> ${target}`)
    }
    expect(bad).toEqual([])
  })

  test('0 production modules carry the board’s example people, files or FIXTURE_ constants (comments aside)', () => {
    const bad: string[] = []
    for (const rel of production) {
      const body = code(readFileSync(join(ROOT, rel), 'utf8'))
      for (const word of EXAMPLE_WORDS) if (word.test(body)) bad.push(`${rel}: ${word.source}`)
    }
    expect(bad).toEqual([])
  })

  test('the example-word check fails on a planted name (and passes the same name in a comment)', () => {
    const hits = (src: string) => EXAMPLE_WORDS.filter((w) => w.test(code(src))).map((w) => w.source)
    expect(hits('<Page title="Anna’s iPad" />')).toEqual([EXAMPLE_WORDS[0].source])
    expect(hits('export const FIXTURE_DEVICES = []')).toEqual([EXAMPLE_WORDS[EXAMPLE_WORDS.length - 1].source])
    expect(hits('/** As the button names it: "Anna’s iPad’s". */\nconst x = 1')).toEqual([])
  })

  test('the one door stays behind the dev-server check', () => {
    const live = readFileSync(join(ROOT, 'src/v2/live.tsx'), 'utf8')
    expect(live).toContain('if (!import.meta.env.DEV) return Window')
    expect(Object.keys(DOORS)).toEqual(['src/v2/live.tsx'])
  })
})
