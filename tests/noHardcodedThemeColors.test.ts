/**
 * Guard against new hard-coded, non-theme-aware colors landing in `src/`
 * (task 1523 — dark mode broke in several places because a color/background/
 * border/fill/stroke value was a literal hex or 'white'/'black' instead of a
 * `var(--*)` design token, so it never adapted when `data-theme` flipped).
 *
 * This is intentionally narrow: it only flags a CSS-color-property assignment
 * (`color:`, `background[-color]:`, `border[-color]:`, `fill=`, `stroke=`)
 * whose VALUE is a literal hex / `white` / `black`, in `.ts`/`.tsx` source
 * under `src/`. It does not touch `design.css` (the token file itself,
 * where literal colors are exactly where they belong) or comment lines
 * (which legitimately cite hex values when explaining what NOT to do, e.g.
 * `Logo.tsx`'s doc comment about the asset's baked-in `#1A1714`). A value
 * unrelated to a color property — e.g. `WindowsTray.tsx`'s
 * `FILE_TYPE_COLORS` map (`pdf: '#dc2626'`), which is a fixed per-file-type
 * accent unrelated to light/dark — is not a color-PROPERTY assignment and is
 * correctly left alone.
 *
 * Mutation check (2026-09-25, reverted after confirming RED): temporarily
 * re-added `background: '#faf8f5'` to `Onboarding.tsx`'s FinderInstallStep
 * card (the exact defect this task fixed) — this test failed, naming the
 * file and line, then passed again once reverted.
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

const SRC_ROOT = join(import.meta.dir, '..', 'src')

// A literal hex color or bare white/black, as the VALUE of a color-ish CSS
// property or SVG presentation attribute. Deliberately does NOT match
// `var(--...)`, `currentColor`, `transparent`, or property names other than
// the color-carrying ones below (so unrelated maps like per-file-type accent
// colors, which are not theme-reactive, are left alone).
const HARDCODED_COLOR_VALUE =
  /\b(?:color|background(?:-color)?|border(?:-color)?|fill|stroke)\s*[:=]\s*['"`]\s*(#[0-9a-fA-F]{3,8}\b|white|black)\b/i

function isCommentLine(line: string): boolean {
  const trimmed = line.trim()
  return trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*')
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

  test('no color/background/border/fill/stroke is a literal hex or white/black', () => {
    const violations: string[] = []
    for (const file of files) {
      const lines = readFileSync(file, 'utf8').split('\n')
      lines.forEach((line, index) => {
        if (isCommentLine(line)) return
        const match = HARDCODED_COLOR_VALUE.exec(line)
        if (match) {
          violations.push(`${relative(SRC_ROOT, file)}:${index + 1}: ${line.trim()}`)
        }
      })
    }
    expect(violations).toEqual([])
  })
})
