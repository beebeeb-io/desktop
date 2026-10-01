/**
 * The layout rules that stop screenshot 3 from coming back, pinned where bun can see them
 * (task 1683 slice 4, spec section 8). tests/render-mac-settings.mjs measures the same rules
 * in Chromium; this file is the part CI runs. It also pins the mount: the window is reachable
 * only at `?window=settings-v2&platform=macos`, and nothing else imports it.
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

const SRC = join(import.meta.dir, '..', 'src')
const read = (file: string) => readFileSync(join(SRC, file), 'utf8').replace(/\r\n/g, '\n')

interface Rule {
  selectors: string[]
  decls: Map<string, string>
}

/** A small CSS reader: top-level rules and the rules inside @media, comments removed. Enough for one flat file. */
function parseCss(source: string): { rules: Rule[]; atRules: string[] } {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, '')
  const rules: Rule[] = []
  const atRules: string[] = []
  const re = /([^{}]+)\{([^{}]*)\}|(@[^{;]+)\{/g
  let match: RegExpExecArray | null
  while ((match = re.exec(text))) {
    if (match[3]) {
      atRules.push(match[3].trim())
      continue
    }
    const decls = new Map<string, string>()
    for (const part of match[2].split(';')) {
      const at = part.indexOf(':')
      if (at > 0) decls.set(part.slice(0, at).trim(), part.slice(at + 1).trim().replace(/\s+/g, ' '))
    }
    rules.push({ selectors: match[1].split(',').map((s) => s.trim().replace(/\s+/g, ' ')), decls })
  }
  return { rules, atRules }
}

const css = parseCss(read('macSettings.css'))
const rulesFor = (selector: string) => css.rules.filter((r) => r.selectors.includes(selector))
const declOf = (selector: string, prop: string) => rulesFor(selector).map((r) => r.decls.get(prop)).find((v) => v !== undefined)

describe('the stylesheet reads as expected (so the checks below are not vacuous)', () => {
  test('parses the shell rule and a rule list', () => {
    expect(css.rules.length).toBeGreaterThan(40)
    expect(declOf('.ms-shell', 'height')).toBe('100vh')
    expect(rulesFor('.ms-tab:focus-visible').length).toBe(1) // a selector list is split
  })
})

describe('screenshot 3, defect 1: the toolbar can never scroll away', () => {
  test('the shell is exactly the window and clips; the rows are the toolbar and the pane', () => {
    expect(declOf('.ms-shell', 'height')).toBe('100vh')
    expect(declOf('.ms-shell', 'overflow')).toBe('hidden')
    expect(declOf('.ms-shell', 'display')).toBe('grid')
    expect(declOf('.ms-shell', 'grid-template-rows')).toBe('auto minmax(0, 1fr)')
  })

  test('the pane is the only scroll container: it scrolls vertically, may shrink, and never scrolls sideways', () => {
    expect(declOf('.ms-pane', 'overflow-y')).toBe('auto')
    expect(declOf('.ms-pane', 'overflow-x')).toBe('hidden')
    expect(declOf('.ms-pane', 'min-height')).toBe('0')
    expect(declOf('.ms-pane', 'min-width')).toBe('0')
  })

  test('no other rule scrolls except the folder list inside the sheet', () => {
    const scrollers = css.rules
      .filter((r) => ['overflow', 'overflow-x', 'overflow-y'].some((p) => ['auto', 'scroll'].includes(r.decls.get(p) ?? '')))
      .flatMap((r) => r.selectors)
      .sort()
    expect(scrollers).toEqual(['.ms-folder-list', '.ms-pane'])
  })

  test('nothing is sticky or fixed in the stylesheet: the toolbar stays put because the document never scrolls', () => {
    const positioned = css.rules.filter((r) => ['sticky', 'fixed'].includes(r.decls.get('position') ?? '')).flatMap((r) => r.selectors)
    expect(positioned).toEqual([])
  })
})

describe('screenshot 3, defect 2: one unbreakable string cannot widen a track', () => {
  test('grid tracks are minmax(0, 1fr), never a bare 1fr', () => {
    expect(declOf('.ms-shell', 'grid-template-columns')).toBe('minmax(0, 1fr)')
    const bare = css.rules.flatMap((r) => [...r.decls].filter(([k, v]) => k.startsWith('grid-template') && /(^|[\s(,])\d*\.?\d*fr\b/.test(v.replace(/minmax\([^)]*\)/g, ''))).map(([k, v]) => `${r.selectors.join(',')} ${k}: ${v}`))
    expect(bare).toEqual([])
  })

  test('every flexible text block (flex: 1) is min-width: 0', () => {
    const flexible = css.rules.filter((r) => /^1( |$)/.test(r.decls.get('flex') ?? ''))
    expect(flexible.flatMap((r) => r.selectors).sort()).toEqual(['.ms-account-text', '.ms-bar', '.ms-note-text', '.ms-row-text'])
    const missing = flexible.filter((r) => r.decls.get('min-width') !== '0').flatMap((r) => r.selectors)
    expect(missing).toEqual([])
  })

  test('every box that holds a row or a card may shrink below its content', () => {
    for (const selector of ['.ms-shell', '.ms-toolbar', '.ms-pane', '.ms-group', '.ms-card', '.ms-row', '.ms-row-control', '.ms-account', '.ms-note', '.ms-storage']) {
      expect({ selector, minWidth: declOf(selector, 'min-width') }).toEqual({ selector, minWidth: '0' })
    }
  })

  test('everything that can hold user data wraps anywhere', () => {
    for (const selector of ['.ms-row-label', '.ms-row-hint', '.ms-account-email', '.ms-account-plan', '.ms-note-title', '.ms-note-body', '.ms-sheet-copy']) {
      expect({ selector, wrap: declOf(selector, 'overflow-wrap') }).toEqual({ selector, wrap: 'anywhere' })
    }
  })

  test('the one-line mono reason is cut with an ellipsis instead of widening anything', () => {
    expect(declOf('.ms-mono', 'white-space')).toBe('nowrap')
    expect(declOf('.ms-mono', 'overflow')).toBe('hidden')
    expect(declOf('.ms-mono', 'text-overflow')).toBe('ellipsis')
  })

  test('there is no width breakpoint: the window is one layout, so two layout models cannot compete', () => {
    expect(css.atRules.filter((a) => /width/.test(a))).toEqual([])
    expect(css.atRules).toEqual(['@media (prefers-reduced-motion: no-preference)'])
  })
})

describe('the sheet and the pane agree with the markup', () => {
  test('every ms- class the components use has a rule (a typo would silently lose its layout)', () => {
    const used = new Set<string>()
    for (const file of ['MacSettings.tsx', 'macSettingsParts.tsx']) {
      for (const attr of read(file).matchAll(/className=(\{[^}]*\}|"[^"]*"|'[^']*')/g)) {
        for (const m of attr[1].matchAll(/['"`]((?:ms-[a-z-]+ ?)+)['"`]/g)) m[1].trim().split(' ').forEach((c) => used.add(c))
      }
    }
    const defined = new Set(css.rules.flatMap((r) => r.selectors).flatMap((s) => [...s.matchAll(/\.(ms-[a-z-]+)/g)].map((m) => m[1])))
    expect([...used].filter((c) => !defined.has(c)).sort()).toEqual([])
    expect(used.size).toBeGreaterThan(25)
  })

  test('a text column in a flex row is never a child without the shrinkable class', () => {
    const parts = read('macSettingsParts.tsx') + read('MacSettings.tsx')
    expect((parts.match(/className="ms-row-text"/g) ?? []).length).toBeGreaterThanOrEqual(2)
    expect(parts).not.toMatch(/style=\{\{[^}]*(whiteSpace|white-space)/)
  })
})

describe('the mount: only at ?window=settings-v2&platform=macos, and nothing renders it today', () => {
  const main = read('main.tsx')

  test('main.tsx mounts MacSettings on exactly one branch, which needs both the window and the platform', () => {
    const branch = main.match(/\}\s*else if \(([^)]*settings-v2[^)]*)\)\s*\{\s*component = <MacSettings \/>/)
    expect(branch).not.toBeNull()
    expect(branch![1]).toContain("which === 'settings-v2'")
    expect(branch![1]).toContain("platform === 'macos'")
    expect((main.match(/<MacSettings/g) ?? []).length).toBe(1)
  })

  test('today\'s default window is still App, on the final else, with its macOS flyout class', () => {
    const tail = main.slice(main.indexOf("component = <MacSettings />"))
    expect(tail).toContain("classList.add('macos-flyout')")
    expect(tail).toContain('component = <App />')
  })

  test('the settings-v2 branch is wrapped like every other app window (session boundary, toasts, capabilities)', () => {
    expect(main).toContain('<AccountSessionBoundary><ToastProvider><CapabilityProvider>{component}')
  })

  test('only main.tsx imports the window and its stylesheet', () => {
    const importers: string[] = []
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir)) {
        const full = join(dir, entry)
        if (statSync(full).isDirectory()) { walk(full); continue }
        if (!/\.(ts|tsx)$/.test(entry)) continue
        if (/^\s*import [^\n]*['"]\.{1,2}\/(?:[\w/]*\/)?(?:MacSettings|macSettings\.css)['"]/m.test(readFileSync(full, 'utf8'))) importers.push(relative(SRC, full).replaceAll('\\', '/'))
      }
    }
    walk(SRC)
    expect(importers).toEqual(['main.tsx'])
  })

  test('the window never asks the browser: no window.confirm, no alert', () => {
    const source = read('MacSettings.tsx')
    expect(source).not.toMatch(/window\.(confirm|alert|prompt)\(/)
  })
})
