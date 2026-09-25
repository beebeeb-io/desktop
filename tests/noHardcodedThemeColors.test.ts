/**
 * Guard against new hard-coded, non-theme-aware colors landing in `src/`
 * (task 1523 — dark mode broke in several places because a color/background/
 * border/fill/stroke value was a literal hex or 'white'/'black' instead of a
 * `var(--*)` design token, so it never adapted when `data-theme` flipped).
 *
 * This is intentionally narrow: it only flags a color-bearing property
 * assignment (`color`, `background[-color]`, `border[-top|-right|-bottom
 * |-left][-color]`, `outline[-color]`, `box-shadow`, `fill`, `stroke` —
 * kebab-case CSS *or* camelCase inline-style/JSX, e.g. `borderColor` /
 * `backgroundColor`) whose VALUE contains a literal hex color, an
 * `rgb()`/`rgba()`/`hsl()`/`hsla()` call, or a bare `white`/`black`, in
 * `.ts`/`.tsx` source under `src/`. The literal can be anywhere inside the
 * quoted value, not just immediately after the opening quote, so a
 * composite value like `border: '1px solid #e5e7eb'` is caught too, not
 * only a bare `border-color: '#e5e7eb'`.
 *
 * It does not touch `design.css` (the token file itself, where literal
 * colors are exactly where they belong) or comment lines (which
 * legitimately cite hex values when explaining what NOT to do, e.g.
 * `Logo.tsx`'s doc comment about the asset's baked-in `#1A1714`). A value
 * unrelated to a color property — e.g. `WindowsTray.tsx`'s
 * `FILE_TYPE_COLORS` map (`pdf: '#dc2626'`), which is a fixed per-file-type
 * accent unrelated to light/dark — is not a color-PROPERTY assignment and is
 * correctly left alone (checked against a property allowlist, not a bare
 * regex on the surrounding text).
 *
 * Mutation check (2026-09-25, reverted after confirming RED): temporarily
 * re-added `background: '#faf8f5'` to `Onboarding.tsx`'s FinderInstallStep
 * card (the exact defect this task fixed) — this test failed, naming the
 * file and line, then passed again once reverted.
 *
 * Mutation check (2026-09-25, code review on PR #45, reverted after
 * confirming RED both times): temporarily inserted `border: '1px solid
 * #e5e7eb'` (composite value, the exact miss the original matcher had) and
 * separately `borderColor: '#e5e7eb'` (camelCase property, the other miss)
 * into `src/Onboarding.tsx` — each failed the test, naming the file and
 * line, then passed again once reverted. See the task's Notes for the
 * pasted command output.
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

const SRC_ROOT = join(import.meta.dir, '..', 'src')

// Color-bearing properties only — kebab-case (CSS) and camelCase
// (inline-style / JSX) variants are unified onto this kebab-case set by
// `toKebabCase()` before the lookup, so both `border-color:` and
// `borderColor:` are covered by one entry. Anything not in this set (e.g.
// `pdf:` in a per-file-type accent map, or `borderRadius:`) is left alone
// regardless of what its value looks like — this is what keeps the
// `FILE_TYPE_COLORS`-style false positive from firing.
const COLOR_BEARING_PROPERTIES = new Set([
  'color',
  'background',
  'background-color',
  'border',
  'border-color',
  'border-top',
  'border-top-color',
  'border-right',
  'border-right-color',
  'border-bottom',
  'border-bottom-color',
  'border-left',
  'border-left-color',
  'outline',
  'outline-color',
  'box-shadow',
  'fill',
  'stroke',
])

// `property: 'value'` or `property="value"` — kebab-case or camelCase
// property name, single/double/backtick-quoted value. Global so a line with
// several assignments (e.g. a JSX `style={{ ... }}` object spread across one
// line) yields every one of them.
const PROPERTY_ASSIGNMENT = /\b([A-Za-z][A-Za-z-]*)\s*[:=]\s*(['"`])((?:(?!\2).)*)\2/g

// A literal color value — hex, an rgb()/rgba()/hsl()/hsla() call, or a bare
// white/black — matched ANYWHERE inside the property's value (not just
// immediately after the opening quote), so composite values like
// '1px solid #e5e7eb' or '0 1px 2px rgba(0,0,0,0.4)' are caught too.
// Deliberately does NOT match `var(--...)`, `currentColor`, or
// `transparent`.
const HARDCODED_COLOR_VALUE = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?)\(|\b(?:white|black)\b/i

// borderTopColor -> border-top-color, background -> background (no-op).
function toKebabCase(prop: string): string {
  return prop.replace(/([a-z0-9])([A-Z])/g, '$1-$2').toLowerCase()
}

function isCommentLine(line: string): boolean {
  const trimmed = line.trim()
  return trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*')
}

// Every color-bearing-property assignment on this line whose value literally
// contains a hardcoded color, as the matched `prop: 'value'` snippets.
function findColorViolations(line: string): string[] {
  const violations: string[] = []
  PROPERTY_ASSIGNMENT.lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = PROPERTY_ASSIGNMENT.exec(line))) {
    const [full, rawProp, , value] = match
    if (!COLOR_BEARING_PROPERTIES.has(toKebabCase(rawProp))) continue
    if (HARDCODED_COLOR_VALUE.test(value)) {
      violations.push(full.trim())
    }
  }
  return violations
}

function listSourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir)) {
    if (entry === 'assets' || entry === 'node_modules') continue
    const full = join(dir, entry)
    const stat = statSync(full)
    if (stat.isDirectory()) {
      out.push(...listSourceFiles(full))
    } else if (/\.(tsx?|css)$/.test(entry) && entry !== 'design.css') {
      out.push(full)
    }
  }
  return out
}

describe('no hard-coded, non-theme-aware colors in src/', () => {
  const files = listSourceFiles(SRC_ROOT)

  test('found at least one source file to scan (the scan itself is not a no-op)', () => {
    expect(files.length).toBeGreaterThan(10)
  })

  test('no color/background/border/outline/box-shadow/fill/stroke is a literal hex, rgb()/hsl(), or white/black', () => {
    const violations: string[] = []
    for (const file of files) {
      const lines = readFileSync(file, 'utf8').split('\n')
      lines.forEach((line, index) => {
        if (isCommentLine(line)) return
        for (const snippet of findColorViolations(line)) {
          violations.push(`${relative(SRC_ROOT, file)}:${index + 1}: ${snippet}`)
        }
      })
    }
    expect(violations).toEqual([])
  })
})
