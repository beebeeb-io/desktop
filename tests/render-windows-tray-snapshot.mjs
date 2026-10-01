// DOM + geometry + computed-colour snapshot of the Windows tray flyout (`?window=tray`),
// task 1683 slice 3 regression guard for the extraction into src/trayShared.tsx
// (spec section 11: "a DOM and geometry snapshot of ?window=tray (element tree, bounding
// boxes, computed colours) is equal before and after").
//
//   node tests/render-windows-tray-snapshot.mjs REPO OUTPUT
//   PLAYWRIGHT_MODULE=/opt/node22/lib/node_modules/playwright/index.mjs
//
// Run it on the commit BEFORE the extraction and on the one AFTER, then compare the two
// `windows-tray-snapshot.json` files byte for byte (`cmp`). Every scenario drives the real
// WindowsTray component through the real `@tauri-apps/api` against the mock bridge; time is
// frozen so the relative-time labels cannot drift between runs. Local evidence, not a CI gate.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, readFile, symlink, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { tauriMockInitScript } from './fixtures/tauriMock.mjs'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-windows-tray-snapshot.mjs REPO OUTPUT')
await mkdir(output, { recursive: true })
// The generated entry lives in OUTPUT, so bare imports (react, react-dom) resolve through this link.
if (!existsSync(path.join(output, 'node_modules'))) await symlink(path.join(repo, 'node_modules'), path.join(output, 'node_modules'))

const entry = path.join(output, 'windows-tray-entry.tsx')
await writeFile(entry, `import { createRoot } from 'react-dom/client'
import WindowsTray from ${JSON.stringify(path.join(repo, 'src/WindowsTray.tsx'))}
import { ToastProvider } from ${JSON.stringify(path.join(repo, 'src/windows/ui.tsx'))}
document.documentElement.classList.add('tray-window')
createRoot(document.getElementById('root')!).render(<ToastProvider><WindowsTray /></ToastProvider>)
`)
const bundlePath = path.join(output, 'windows-tray.browser.js')
const built = spawnSync('bun', ['build', entry, '--outfile', bundlePath, '--target', 'browser', '--format', 'iife'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)
const css = await readFile(path.join(repo, 'src/design.css'), 'utf8')

const NOW = 1_790_000_000_000
const recent = (n) => Array.from({ length: n }, (_, i) => ({
  path: `/Users/fixture/Beebeeb/${['Docs', 'Photos', 'Team'][i % 3]}/${['report.pdf', 'IMG_01.heic', 'notes.md', 'clip.mov', 'song.mp3', 'a.zip', 'main.rs', 'plain.txt'][i % 8]}`,
  size_bytes: 1000 * (i + 1),
  status: ['local', 'uploading', 'cloud_only', 'conflict', 'error', 'downloading', 'moved_to_trash', 'local'][i % 8],
  activity_type: i % 8 === 6 ? 'moved_to_trash' : null,
  modified_at: Math.floor(NOW / 1000) - 60 * (i + 1) * (i + 1),
}))
const scenarios = {
  loggedIn12: { status: { logged_in: true, engine: 'running', sync_root: '/Users/fixture/Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }, overview: { recent: recent(12) } },
  syncingFewFiles: { status: { logged_in: true, engine: 'running', sync_root: '/Users/fixture/Beebeeb', syncing: 3, cloud_only: 0, conflicts: 2 }, overview: { recent: recent(4) } },
  noRoot: { status: { logged_in: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }, overview: { recent: [] } },
  signedOut: { status: { logged_in: false, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }, overview: null },
}

const PROPS = ['display', 'position', 'boxSizing', 'width', 'height', 'margin', 'padding', 'gap', 'flex', 'alignItems', 'justifyContent', 'gridTemplateColumns', 'color', 'backgroundColor',
  'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'borderTopColor', 'borderLeftColor', 'borderRadius', 'fontFamily', 'fontSize', 'fontWeight', 'lineHeight',
  'letterSpacing', 'textAlign', 'whiteSpace', 'overflow', 'textOverflow', 'opacity', 'cursor', 'fill', 'stroke', 'strokeWidth']

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
const out = {}
try {
  for (const [name, sc] of Object.entries(scenarios)) {
    const page = await browser.newPage({ viewport: { width: 400, height: 560 } })
    const errors = []
    page.on('pageerror', (e) => errors.push(String(e)))
    page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()) })
    await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div></body></html>')
    await page.addStyleTag({ content: css })
    // Installed with evaluate, not addInitScript: setContent does not navigate, so init scripts do not run.
    await page.evaluate(`Date.now = () => ${NOW}`)
    await page.evaluate(tauriMockInitScript())
    await page.evaluate(({ sc }) => {
      window.__handler = async (cmd) => {
        if (cmd === 'sync_status') return sc.status
        if (cmd === 'desktop_file_overview') { if (!sc.overview) throw new Error('vault locked'); return sc.overview }
        throw new Error('unhandled ' + cmd)
      }
    }, { sc })
    await page.addScriptTag({ path: bundlePath })
    await page.waitForSelector('button[aria-label="Open settings"]', { timeout: 5000 }).catch((e) => { throw new Error('flyout did not render; page errors: ' + JSON.stringify(errors) + ' ' + e.message) })
    await page.waitForFunction(() => window.__appCalls().some((c) => c.cmd === 'desktop_file_overview'))
    await page.waitForTimeout(300)
    await page.evaluate(() => document.fonts.ready)
    const tree = await page.evaluate((PROPS) => {
      const root = document.getElementById('root')
      const walk = (el, depth) => {
        const r = el.getBoundingClientRect()
        const cs = getComputedStyle(el)
        const style = {}
        for (const p of PROPS) style[p] = cs[p]
        const attrs = {}
        for (const a of el.attributes) attrs[a.name] = a.value
        return {
          tag: el.tagName.toLowerCase(), attrs, depth, text: el.children.length === 0 ? (el.textContent || '') : '',
          rect: [r.x, r.y, r.width, r.height].map((n) => Math.round(n * 100) / 100), style,
          children: [...el.children].map((c) => walk(c, depth + 1)),
        }
      }
      return walk(root, 0)
    }, PROPS)
    // Hover is part of the look: the extraction moved the mouse handlers, so move the mouse.
    const hover = {}
    for (const [label, locator] of [['gear', page.locator('button[aria-label="Open settings"]')], ['footer0', page.locator('button', { hasText: 'Open folder' })],
      ['footer1', page.locator('button', { hasText: 'View online' })], ['footer2', page.locator('button', { hasText: 'Recycle bin' })]]) {
      await locator.hover({ force: true })
      hover[label] = await locator.evaluate((el) => getComputedStyle(el).backgroundColor)
    }
    tree.hover = hover
    const count = (n) => (n.children.length ? 1 + n.children.reduce((s, c) => s + count(c), 0) : 1)
    out[name] = { elements: count(tree), tree, errors }
    assert.deepEqual(errors, [], `${name}: page errors`)
    assert(out[name].elements > 20, `${name}: only ${out[name].elements} elements, the flyout did not render`)
    await page.close()
  }
} finally { await browser.close() }
await writeFile(path.join(output, 'windows-tray-snapshot.json'), JSON.stringify(out, null, 1))
const counts = Object.entries(out).map(([n, v]) => `${n}=${v.elements}`).join(' ')
console.log(`windows tray snapshot: ${Object.keys(out).length} scenarios, elements ${counts}`)
