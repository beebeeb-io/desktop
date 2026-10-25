/**
 * The layout rules that stop screenshot 3 from coming back, pinned where bun can see them
 * (task 1683 slice 4, spec section 8). tests/render-mac-settings.mjs measures the same rules
 * in Chromium; this file is the part CI runs. It also pins the mount.
 *
 * Task 1683 slice 1 (two-surface spec §7, ruling 1): the Settings sections now live in the Beebeeb window
 * (`MacApp.tsx`, `macApp.css`), whose sidebar list replaced the toolbar tab strip. The shell rules that pinned the
 * Settings window (`.ms-shell`, `.ms-toolbar`) moved to the window shell (`.mw-window`, `.mw-main`); every rule about
 * the rows, the pane and wrapping is unchanged, and the same rules are pinned for `macApp.css`. The window is mounted
 * at `?window=main&platform=macos` only.
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
const win = parseCss(read('macApp.css'))
const winDecl = (selector: string, prop: string) =>
  win.rules.filter((r) => r.selectors.includes(selector)).map((r) => r.decls.get(prop)).find((v) => v !== undefined)

describe('the stylesheet reads as expected (so the checks below are not vacuous)', () => {
  test('parses the shell rule and a rule list', () => {
    expect(css.rules.length).toBeGreaterThan(40)
    expect(declOf('.ms-pane', 'overflow-y')).toBe('auto')
    expect(rulesFor('.ms-btn:focus-visible').length).toBe(1) // a selector list is split
    expect(win.rules.length).toBeGreaterThan(30)
    expect(winDecl('.mw-window', 'height')).toBe('100vh')
  })
})

describe('screenshot 3, defect 1: the toolbar can never scroll away', () => {
  test('the window is exactly the viewport and clips; its columns are the 214 pt sidebar and the pane', () => {
    expect(winDecl('.mw-window', 'height')).toBe('100vh')
    expect(winDecl('.mw-window', 'overflow')).toBe('hidden')
    expect(winDecl('.mw-window', 'display')).toBe('grid')
    expect(winDecl('.mw-window', 'grid-template-columns')).toBe('214px minmax(0, 1fr)')
    expect(winDecl('.mw-window', 'grid-template-rows')).toBe('minmax(0, 1fr)')
    // The pane's rows are the 52 pt toolbar and the content (spec §7.1): the toolbar can never scroll away.
    expect(winDecl('.mw-main', 'grid-template-rows')).toBe('52px minmax(0, 1fr)')
    expect(winDecl('.mw-main', 'min-height')).toBe('0')
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
    const windowScrollers = win.rules
      .filter((r) => ['overflow', 'overflow-x', 'overflow-y'].some((p) => ['auto', 'scroll'].includes(r.decls.get(p) ?? '')))
      .flatMap((r) => r.selectors)
      .sort()
    // Home's content and the signed-out card; the Settings pane is `.ms-pane` above.
    expect(windowScrollers).toEqual(['.mw-content', '.mw-solo-body'])
  })

  test('nothing is sticky or fixed in the stylesheet: the toolbar stays put because the document never scrolls', () => {
    const positioned = [...css.rules, ...win.rules].filter((r) => ['sticky', 'fixed'].includes(r.decls.get('position') ?? '')).flatMap((r) => r.selectors)
    expect(positioned).toEqual([])
  })
})

describe('screenshot 3, defect 2: one unbreakable string cannot widen a track', () => {
  test('grid tracks are minmax(0, 1fr), never a bare 1fr', () => {
    expect(winDecl('.mw-window', 'grid-template-columns')).toBe('214px minmax(0, 1fr)')
    const bare = [...css.rules, ...win.rules].flatMap((r) => [...r.decls].filter(([k, v]) => k.startsWith('grid-template') && /(^|[\s(,])\d*\.?\d*fr\b/.test(v.replace(/minmax\([^)]*\)/g, ''))).map(([k, v]) => `${r.selectors.join(',')} ${k}: ${v}`))
    expect(bare).toEqual([])
  })

  test('every flexible text block (flex: 1) is min-width: 0', () => {
    const flexible = css.rules.filter((r) => /^1( |$)/.test(r.decls.get('flex') ?? ''))
    expect(flexible.flatMap((r) => r.selectors).sort()).toEqual(['.ms-account-text', '.ms-bar', '.ms-note-text', '.ms-row-text'])
    const missing = flexible.filter((r) => r.decls.get('min-width') !== '0').flatMap((r) => r.selectors)
    expect(missing).toEqual([])
    const windowFlexible = win.rules.filter((r) => /^1( |$)/.test(r.decls.get('flex') ?? ''))
    expect(windowFlexible.flatMap((r) => r.selectors).sort()).toEqual(['.mw-account-text', '.mw-row-text'])
    expect(windowFlexible.filter((r) => r.decls.get('min-width') !== '0').flatMap((r) => r.selectors)).toEqual([])
  })

  test('every box that holds a row or a card may shrink below its content', () => {
    for (const selector of ['.ms-pane', '.ms-group', '.ms-card', '.ms-row', '.ms-row-control', '.ms-account', '.ms-note', '.ms-storage']) {
      expect({ selector, minWidth: declOf(selector, 'min-width') }).toEqual({ selector, minWidth: '0' })
    }
    for (const selector of ['.mw-window', '.mw-sidebar', '.mw-main', '.mw-toolbar', '.mw-content', '.mw-card', '.mw-row', '.mw-columns', '.mw-nav', '.mw-account-row', '.mw-foot']) {
      expect({ selector, minWidth: winDecl(selector, 'min-width') }).toEqual({ selector, minWidth: '0' })
    }
  })

  test('everything that can hold user data wraps anywhere', () => {
    for (const selector of ['.ms-row-label', '.ms-row-hint', '.ms-account-email', '.ms-account-plan', '.ms-note-title', '.ms-note-body', '.ms-sheet-copy']) {
      expect({ selector, wrap: declOf(selector, 'overflow-wrap') }).toEqual({ selector, wrap: 'anywhere' })
    }
    for (const selector of ['.mw-email', '.mw-plan', '.mw-title', '.mw-row-title', '.mw-row-hint', '.mw-signin-copy', '.mw-signin-foot']) {
      expect({ selector, wrap: winDecl(selector, 'overflow-wrap') }).toEqual({ selector, wrap: 'anywhere' })
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
    expect(win.atRules).toEqual([])
  })
})

describe('the sheet and the pane agree with the markup', () => {
  test('every ms- and mw- class the components use has a rule (a typo would silently lose its layout)', () => {
    const used = new Set<string>()
    for (const file of ['MacSettings.tsx', 'macSettingsParts.tsx', 'keptFolderRows.tsx', 'MacApp.tsx']) {
      for (const attr of read(file).matchAll(/className=(\{[^}]*\}|"[^"]*"|'[^']*')/g)) {
        for (const m of attr[1].matchAll(/['"`]((?:m[sw]-[a-z-]+ ?)+)['"`]/g)) m[1].trim().split(' ').forEach((c) => used.add(c))
        for (const m of attr[1].matchAll(/(m[sw]-[a-z-]+--)\$\{/g)) used.add(`${m[1]}ok`) // a modifier built from a tone: ok is one of them
      }
    }
    const defined = new Set([...css.rules, ...win.rules].flatMap((r) => r.selectors).flatMap((s) => [...s.matchAll(/\.(m[sw]-[a-z-]+)/g)].map((m) => m[1])))
    expect([...used].filter((c) => !defined.has(c)).sort()).toEqual([])
    expect(used.size).toBeGreaterThan(45)
  })

  test('a text column in a flex row is never a child without the shrinkable class', () => {
    const parts = read('macSettingsParts.tsx') + read('MacSettings.tsx') + read('MacApp.tsx') + read('keptFolderRows.tsx')
    expect((parts.match(/className="ms-row-text"/g) ?? []).length).toBeGreaterThanOrEqual(2)
    expect(parts).not.toMatch(/style=\{\{[^}]*(whiteSpace|white-space)/)
  })
})

describe('the mount: the Beebeeb window at ?window=main&platform=macos only (task 1683 slice 1)', () => {
  const main = read('main.tsx')

  test('main.tsx mounts MacApp on exactly one branch, which needs both the window and the platform', () => {
    const branch = main.match(/\}\s*else if \(([^)]*'main'[^)]*)\)\s*\{\s*component = <MacApp \/>/)
    expect(branch).not.toBeNull()
    expect(branch![1]).toContain("which === 'main'")
    expect(branch![1]).toContain("platform === 'macos'")
    expect((main.match(/<MacApp/g) ?? []).length).toBe(1)
    // The retired Settings window's entry is gone: no URL mounts the sections on their own.
    expect(main).not.toContain('settings-v2')
    expect(main).not.toContain('<MacSettings')
  })

  test('today\'s default window is still App, on the final else, with its macOS flyout class (the tray flyout until slice 3)', () => {
    const tail = main.slice(main.indexOf('component = <MacApp />'))
    expect(tail).toContain("classList.add('macos-flyout')")
    expect(tail).toContain('component = <App />')
  })

  test('the main branch is wrapped like every other app window (session boundary, toasts, capabilities)', () => {
    expect(main).toContain('<AccountSessionBoundary><ToastProvider><CapabilityProvider>{component}')
  })

  test('only main.tsx imports the window and the stylesheets; only the window imports the Settings sections', () => {
    const importersOf = (pattern: RegExp) => {
      const found: string[] = []
      const walk = (dir: string) => {
        for (const entry of readdirSync(dir)) {
          const full = join(dir, entry)
          if (statSync(full).isDirectory()) { walk(full); continue }
          if (!/\.(ts|tsx)$/.test(entry)) continue
          if (pattern.test(readFileSync(full, 'utf8'))) found.push(relative(SRC, full).replaceAll('\\', '/'))
        }
      }
      walk(SRC)
      return found.sort()
    }
    expect(importersOf(/^\s*import [^\n]*['"]\.{1,2}\/(?:[\w/]*\/)?(?:MacApp|macApp\.css|macSettings\.css)['"]/m)).toEqual(['main.tsx'])
    // Security is a Settings section in a file of its own (slice 5) and reuses the sheet the other sections use.
    expect(importersOf(/^\s*import [^\n]*['"]\.{1,2}\/(?:[\w/]*\/)?MacSettings['"]/m)).toEqual(['MacApp.tsx', 'MacSecurity.tsx'])
  })

  test('the window never asks the browser: no window.confirm, no alert', () => {
    for (const file of ['MacSettings.tsx', 'MacApp.tsx']) expect(read(file)).not.toMatch(/window\.(confirm|alert|prompt)\(/)
  })
})
