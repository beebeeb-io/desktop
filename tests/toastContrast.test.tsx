/**
 * Toast contrast (task 1683 slice 5, problem P6 — "toast title unreadable", screenshot 1).
 *
 * What is asserted: for each of the four toast variants, in each of the three theme scopes
 * `design.css` defines (light, `[data-theme='dark']`, and the `prefers-color-scheme: dark`
 * media block), the colours the <Toast> component ACTUALLY RENDERS satisfy
 *   - title, message, close glyph, action-button label  >= 4.5:1  (WCAG AA text)
 *   - icon disc against the toast surface, and its glyph against the disc  >= 3:1
 * and every one of those colours is a `var(--token)`, never a literal. The colours are read
 * back out of `renderToStaticMarkup(<Toast/>)`, not out of the tone helper, so the test is
 * about what reaches the screen. Tokens are resolved from the real `src/design.css`.
 *
 * The instrument is pinned first (see "instrument" below): it must reproduce the numbers
 * recorded in /home/user/evidence/1683/contrast.txt, including the 1.10:1 of the pre-fix
 * dark error title, before its verdicts on the toasts mean anything.
 *
 * Mutation checks live in the task's Notes (Notes 2026-09-30 slice 5): restore the pre-fix
 * hard-coded tones -> this file fails naming the variant, theme and ratio.
 */
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { Toast, type ToastRecord, type ToastVariant } from '../src/windows/ui'
import { THEMES, contrast, ruleDeclarations, tokensFor, type Theme } from './fixtures/themeTokens'

const VARIANTS: ToastVariant[] = ['info', 'success', 'warning', 'error']
const TITLE = 'Couldn’t save your setting'
const MESSAGE = 'The change did not reach the server.'
const ACTION = 'Try again'

function render(variant: ToastVariant): string {
  const toast: ToastRecord = {
    id: `t-${variant}`,
    title: TITLE,
    message: MESSAGE,
    variant,
    action: { label: ACTION, onClick: () => {} },
    durationMs: null,
    dismissible: true,
    createdAt: 0,
  }
  return renderToStaticMarkup(<Toast toast={toast} onDismiss={() => {}} />)
}

function styleOf(html: string, pattern: RegExp, what: string): Record<string, string> {
  const match = pattern.exec(html)
  if (!match) throw new Error(`toast markup has no ${what}:\n${html}`)
  const style: Record<string, string> = {}
  for (const declaration of match[1].split(';')) {
    const colon = declaration.indexOf(':')
    if (colon > 0) style[declaration.slice(0, colon).trim()] = declaration.slice(colon + 1).trim()
  }
  return style
}

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/’/g, '(?:’|&#x27;)')

/** The colours a rendered toast puts on screen, as raw CSS values. */
function renderedColours(variant: ToastVariant) {
  const html = render(variant)
  const root = styleOf(html, /^<div role="(?:alert|status)"[^>]*? style="([^"]*)"/, 'root element')
  const icon = styleOf(html, /<span aria-hidden="true" style="([^"]*)"/, 'icon disc')
  const title = styleOf(html, new RegExp(`<div style="([^"]*)">${escape(TITLE)}</div>`), 'title')
  const message = styleOf(html, new RegExp(`<div style="([^"]*)">${escape(MESSAGE)}</div>`), 'message')
  const close = styleOf(html, /aria-label="Dismiss notification" style="([^"]*)"/, 'close button')
  const action = styleOf(html, new RegExp(`<button type="button" style="([^"]*)">${escape(ACTION)}</button>`), 'action button')
  return {
    surface: root.background,
    border: (root.border ?? '').replace(/^1px solid /, ''),
    title: title.color,
    message: message.color,
    close: close.color,
    iconDisc: icon.background,
    iconGlyph: icon.color,
    actionText: action.color,
    actionBg: action.background,
  }
}

describe('instrument: the contrast measure reproduces the recorded design-phase numbers', () => {
  // Numbers from /home/user/evidence/1683/contrast.txt (contrast.mjs), same formula.
  const tolerance = 0.011
  test('light ink/paper 18.02 and dark ink/paper 16.04 from the real design.css tokens', () => {
    const light = tokensFor('light')
    const dark = tokensFor('dark-explicit')
    expect(Math.abs(contrast('var(--ink)', 'var(--paper)', light) - 18.02)).toBeLessThan(tolerance)
    expect(Math.abs(contrast('var(--ink)', 'var(--paper)', dark) - 16.04)).toBeLessThan(tolerance)
  })
  test('the pre-fix dark error title measured 1.10:1 (near-white ink on hard-coded pale pink)', () => {
    const dark = tokensFor('dark-explicit')
    expect(Math.abs(contrast('var(--ink)', 'oklch(0.98 0.02 25)', dark) - 1.1)).toBeLessThan(0.006)
  })
  test('the explicit-dark and media-dark scopes agree on every token they both define', () => {
    const explicit = tokensFor('dark-explicit')
    const media = tokensFor('dark-media')
    const disagreements = Object.keys(explicit).filter((name) => media[name] !== undefined && media[name] !== explicit[name])
    // `--ring` legitimately differs (0.9 vs 0.85 alpha); it is not a toast colour.
    expect(disagreements.filter((name) => name !== '--ring')).toEqual([])
  })
})

describe('every toast colour is a theme token, and readable in light and dark', () => {
  const checks: string[] = []

  for (const variant of VARIANTS) {
    test(`${variant}: every rendered colour is a var(--token), not a literal`, () => {
      const c = renderedColours(variant)
      for (const [part, value] of Object.entries(c)) {
        expect(`${variant}.${part}=${value}`).toMatch(/=var\(--[a-z0-9-]+\)$/)
      }
    })

    for (const theme of THEMES) {
      test(`${variant} in ${theme}: text >= 4.5:1, icon >= 3:1`, () => {
        const tokens = tokensFor(theme as Theme)
        const c = renderedColours(variant)
        const measured: Array<[string, number, number]> = [
          ['title on surface', contrast(c.title, c.surface, tokens), 4.5],
          ['message on surface', contrast(c.message, c.surface, tokens), 4.5],
          ['close glyph on surface', contrast(c.close, c.surface, tokens), 4.5],
          ['action label on action button', contrast(c.actionText, c.actionBg, tokens), 4.5],
          ['icon disc on surface', contrast(c.iconDisc, c.surface, tokens), 3],
          ['icon glyph on disc', contrast(c.iconGlyph, c.iconDisc, tokens), 3],
        ]
        const failures = measured
          .filter(([, ratio, floor]) => ratio < floor)
          .map(([what, ratio, floor]) => `${variant}/${theme}: ${what} is ${ratio.toFixed(2)}:1, needs ${floor}:1`)
        for (const [what] of measured) checks.push(`${variant}/${theme}/${what}`)
        expect(failures).toEqual([])
      })
    }
  }

  test('the loop above measured 4 variants x 3 themes x 6 pairs = 72 pairs (a check that measured nothing is a RED)', () => {
    expect(checks.length).toBe(72)
    expect(new Set(checks).size).toBe(72)
  })
})

describe('the inline error banner (.notice.error) is the same token pair as the error toast', () => {
  const rule = ruleDeclarations('.notice.error')

  test('border, background and text are var(--token), not literals', () => {
    expect(rule['border-color']).toBe('var(--err-line)')
    expect(rule.background).toBe('var(--err-bg)')
    expect(rule.color).toMatch(/^var\(--[a-z0-9-]+\)$/)
  })

  for (const theme of THEMES) {
    test(`text >= 4.5:1 on its background in ${theme}`, () => {
      const ratio = contrast(rule.color, rule.background, tokensFor(theme as Theme))
      expect(ratio).toBeGreaterThanOrEqual(4.5)
    })
  }
})
