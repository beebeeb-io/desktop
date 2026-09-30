/**
 * Theme-token resolution and WCAG contrast, for tests only (task 1683 slice 5).
 *
 * WHY THIS EXISTS: a toast's readability is decided by the pair (foreground token,
 * background token) in a specific theme, not by either colour alone. Task 1683's design
 * phase measured the pre-fix error toast at 1.10:1 in dark (title `var(--ink)` near white
 * on a hard-coded pale pink) — a defect no per-colour lint can see. So the contrast tests
 * resolve every rendered `var(--x)` against the real `src/design.css` blocks, in each
 * theme scope, and compute WCAG 2 relative-luminance contrast from the result.
 *
 * The maths mirrors `/home/user/evidence/1683/contrast.mjs` (OKLCH -> linear sRGB with a
 * hard gamut clamp -> WCAG luminance). `themeTokens.test.ts`-style sanity checks in
 * `toastContrast.test.tsx` pin it to the numbers that evidence file recorded, so a wrong
 * instrument fails loudly instead of quietly blessing or rejecting colours.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

export type Theme = 'light' | 'dark-explicit' | 'dark-media'
export type TokenMap = Record<string, string>

const DESIGN_CSS = readFileSync(join(import.meta.dir, '..', '..', 'src', 'design.css'), 'utf8')

/** Text between the braces of the first block whose header starts at `header`. */
function blockBody(css: string, header: string): string {
  const start = css.indexOf(header)
  if (start < 0) throw new Error(`design.css has no block "${header}"`)
  const open = css.indexOf('{', start)
  let depth = 0
  for (let i = open; i < css.length; i++) {
    if (css[i] === '{') depth++
    else if (css[i] === '}' && --depth === 0) return css.slice(open + 1, i)
  }
  throw new Error(`design.css block "${header}" is not closed`)
}

function declarations(body: string): TokenMap {
  const out: TokenMap = {}
  // Strip block comments first so a `;` or `--x:` inside a comment is never parsed.
  for (const m of body.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    out[m[1]] = m[2].trim()
  }
  return out
}

/** Every custom property visible in `theme`, after the cascade (dark scopes inherit `:root`). */
export function tokensFor(theme: Theme): TokenMap {
  const light = declarations(blockBody(DESIGN_CSS, ':root {'))
  if (theme === 'light') return light
  if (theme === 'dark-explicit') return { ...light, ...declarations(blockBody(DESIGN_CSS, ":root[data-theme='dark']")) }
  const media = blockBody(DESIGN_CSS, '@media (prefers-color-scheme: dark)')
  return { ...light, ...declarations(blockBody(media, ":root:not([data-theme='light'])")) }
}

/** `property: value` declarations of the first design.css rule whose selector is exactly `selector`. */
export function ruleDeclarations(selector: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const m of blockBody(DESIGN_CSS, `${selector} {`).replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([a-z-]+)\s*:\s*([^;]+);/gi)) {
    out[m[1]] = m[2].trim()
  }
  return out
}

export const THEMES: Theme[] = ['light', 'dark-explicit', 'dark-media']

type Oklch = [number, number, number]

function parseOklch(value: string): Oklch | null {
  const m = /^oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)$/.exec(value.trim())
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null
}

/** Resolve `var(--x)` chains and `oklch(L C h)` to an OKLCH triple. Anything else throws. */
export function resolveColor(value: string, tokens: TokenMap, depth = 0): Oklch {
  if (depth > 8) throw new Error(`var() chain too deep: ${value}`)
  const v = value.trim()
  const ref = /^var\(\s*(--[a-z0-9-]+)\s*\)$/i.exec(v)
  if (ref) {
    const next = tokens[ref[1]]
    if (next === undefined) throw new Error(`token ${ref[1]} is not defined in this theme`)
    return resolveColor(next, tokens, depth + 1)
  }
  const direct = parseOklch(v)
  if (!direct) throw new Error(`cannot resolve colour "${value}" (only var(--token) and oklch(L C h) are understood)`)
  return direct
}

function linearSrgb([L, C, h]: Oklch): [number, number, number] {
  const a = C * Math.cos((h * Math.PI) / 180)
  const b = C * Math.sin((h * Math.PI) / 180)
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map((channel) => Math.min(1, Math.max(0, channel))) as [number, number, number]
}

function luminance(color: Oklch): number {
  const [r, g, b] = linearSrgb(color)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** WCAG 2 contrast ratio of two colour expressions in a theme's token set. */
export function contrast(fg: string, bg: string, tokens: TokenMap): number {
  const a = luminance(resolveColor(fg, tokens))
  const b = luminance(resolveColor(bg, tokens))
  const [hi, lo] = a > b ? [a, b] : [b, a]
  return (hi + 0.05) / (lo + 0.05)
}
