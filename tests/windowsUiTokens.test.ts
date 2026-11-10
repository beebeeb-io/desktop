/**
 * The Windows scenes that draw Windows' own UI (the NSIS installer, the sign-in approval toast, the Windows Hello
 * prompt) used to carry their greys, the Windows blue and the progress green as arbitrary Tailwind classes such as
 * `text-[#1a1a1a]`. Those literals never reach `design.css` and are not seen by `noHardcodedThemeColors.test.ts`
 * (a class string is not a color-bearing property assignment), so nothing kept them honest. This slice moves all 34
 * of them into `--color-win-*` tokens: Windows' own colours stay Windows' own colours, but they live in the token
 * file with a comment naming the surface they draw.
 *
 * The three scenes here own 34 of the 39 classes (task #270); `src/v2/onboarding/views.tsx`'s five are a follow-up.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = join(import.meta.dir, '..')
const DESIGN_CSS = join(ROOT, 'src', 'v2', 'design.css')

const SCENES = [
  'src/v2/onboarding/Installer.tsx',
  'src/v2/auth/Approve.tsx',
  'src/v2/auth/parts.tsx',
] as const

// The 19 tokens this slice adds, with exactly today's value: the look must not change.
const NEW_WIN_TOKENS: Record<string, string> = {
  'win-ink-strong': '#1a1a1a',
  'win-ink-3': '#5c5c5c',
  'win-line-2': '#d0d0d0',
  'win-accent': '#0067c0',
  'win-title': '#f3f3f3',
  'win-surface-2': '#f0f0f0',
  'win-button-line-3': '#c8c8c8',
  'win-line-3': '#e0e0e0',
  'win-line-soft': '#d6d6d6',
  'win-line-accent': '#c9cdd2',
  'win-line-strong': '#888888',
  'win-ink-dim': '#a0a0a0',
  'win-tab': '#eef0f3',
  'win-row': '#ebebeb',
  'win-panel': '#e6e6e6',
  'win-desktop-2': '#dfe3e8',
  'win-button-line-2': '#adadad',
  'win-progress-track': '#bcbcbc',
  'win-progress-fill': '#06b025',
}

// A Tailwind arbitrary colour class: `text-[#1a1a1a]`, `bg-[#888]`, `border-[#0067c0]`.
const HEX_CLASS = /[a-z][a-z0-9-]*-\[#[0-9a-fA-F]{3,8}\]/g

/** Every arbitrary hex-colour class in `source` (e.g. `text-[#123456]`), in order. */
export function hexColorClasses(source: string): string[] {
  return source.match(HEX_CLASS) ?? []
}

// A Tailwind utility whose colour is a `win-` token: `bg-win-title`, `border-win-line-2`, `text-win-ink-3`.
const WIN_CLASS = /\b(?:bg|text|border|ring|from|to|via|fill|stroke|outline|decoration|divide|placeholder|caret|accent)-win-[a-z0-9-]+/g

/** The token names behind every `*-win-*` class in `source`, e.g. `win-line-2`, deduplicated. */
export function winTokenNames(source: string): string[] {
  return [...new Set([...source.matchAll(WIN_CLASS)].map((m) => m[0].replace(/^[a-z]+-/, '')))]
}

/** Token names declared with `--color-<name>: <value>;` in the design.css token file. */
function declaredWinTokens(css: string): Map<string, string> {
  const found = new Map<string, string>()
  for (const line of css.split('\n')) {
    const m = line.match(/--color-(win-[a-z0-9-]+):\s*([^;]+);/)
    if (m) found.set(m[1], m[2].trim())
  }
  return found
}

describe('Windows scenes draw Windows with the design tokens', () => {
  test('(a) no [#hex] colour classes in the three Windows scenes', () => {
    const violations = SCENES.flatMap((file) =>
      hexColorClasses(readFileSync(join(ROOT, file), 'utf8')).map((cls) => `${file}: ${cls}`),
    )
    expect(violations).toEqual([])
  })

  test('(b) every *-win-* class used resolves to a --color-win-* token in src/v2/design.css, at its exact value', () => {
    const declared = declaredWinTokens(readFileSync(DESIGN_CSS, 'utf8'))
    const used = new Set(SCENES.flatMap((file) => winTokenNames(readFileSync(join(ROOT, file), 'utf8'))))
    expect(used.size).toBeGreaterThan(0)
    const missing = [...used].filter((name) => !declared.has(name)).map((name) => `--color-${name} is used but not defined`)
    expect(missing).toEqual([])
    const wrongValue = Object.entries(NEW_WIN_TOKENS)
      .filter(([name, value]) => declared.get(name) !== value)
      .map(([name, value]) => `--color-${name}: expected ${value}, design.css has ${declared.get(name) ?? 'nothing'}`)
    expect(wrongValue).toEqual([])
  })

  test('(c) the hex-colour-class scanner can fail: a planted text-[#123456] is caught', () => {
    expect(hexColorClasses('<span className="text-[#123456]" />')).toEqual(['text-[#123456]'])
    expect(hexColorClasses('<span className="bg-win-title text-ink" />')).toEqual([])
  })

  test("(d) each new token's comment names the Windows surface it draws, in one Windows' own UI block", () => {
    const css = readFileSync(DESIGN_CSS, 'utf8')
    expect(css).toContain("Windows' own UI (data-system-drawn / data-desktop)")
    expect(css).toMatch(/not\s+Beebeeb brand colours/)
    const surface = /\b(installer|setup|approval|toast|hello|security|prompt|dialog|desktop|taskbar|progress|panel|window|button|footer|divider|border|track|fill|glyph|text|tab|row)\b/i
    const unnamed = Object.keys(NEW_WIN_TOKENS).filter((name) => {
      const line = css.split('\n').find((l) => l.includes(`--color-${name}:`))
      const comment = line?.match(/\/\*\s*(.+?)\s*\*\//)?.[1]
      return !comment || !surface.test(comment)
    })
    expect(unnamed).toEqual([])
  })
})
