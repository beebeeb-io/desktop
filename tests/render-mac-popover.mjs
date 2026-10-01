// Local Playwright rung for the macOS menu-bar popover (task 1683 slice 3; spec section 12 row 3).
//
//   node tests/render-mac-popover.mjs REPO OUTPUT [DESIGN_PNG_DIR]
//   PLAYWRIGHT_MODULE=/opt/node22/lib/node_modules/playwright/index.mjs
//   FONTS_CSS=/home/user/evidence/1683/fonts.css   (Inter + JetBrains Mono, as in the design renders)
//   AXE_JS=/path/to/axe.min.js                     (default: the bun cache copy; not a dependency)
//   REQUIRE_FONTS=1 / REQUIRE_AXE=1                make a missing font / axe an error
//
// It bundles the PRODUCTION entry (`src/main.tsx`, so the real `?window=popover&platform=macos`
// routing is what is exercised), loads it in Chromium against a scripted stand-in for the Tauri
// bridge (tests/fixtures/tauriMock.mjs), and drives the real component, controller and
// `@tauri-apps/api` calls with Rust's events (`popover-shown`, `engine-status`).
//
// Per state it asserts: the box is 372 x 488; no scroll container overflows horizontally and no
// element spills out of the popover; no axe violation; and it records the title's y. Across the
// message states the title y must be identical. It then checks keyboard order, Esc, the gear,
// and that NOTHING is invoked while the popover is hidden. One PNG per state is saved under
// OUTPUT/png and, when DESIGN_PNG_DIR is given, put beside the design's render with a pixel diff
// under OUTPUT/compare. Local evidence, not a CI gate (Playwright is not a desktop dependency).
// Prints a count line: `N passed, 0 failed`.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, readFile, symlink, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { tauriMockInitScript } from './fixtures/tauriMock.mjs'
import { NOW, SNAPSHOTS, ME, base_snapshot } from './fixtures/macPopoverStates.mjs'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output, designDir] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-mac-popover.mjs REPO OUTPUT [DESIGN_PNG_DIR]')
await mkdir(path.join(output, 'png'), { recursive: true })
await mkdir(path.join(output, 'compare'), { recursive: true })
if (!existsSync(path.join(output, 'node_modules'))) await symlink(path.join(repo, 'node_modules'), path.join(output, 'node_modules'))

let passed = 0
const failures = []
const check = (cond, label) => {
  if (cond) passed += 1
  else failures.push(label)
}
const eq = (actual, expected, label) => {
  if (JSON.stringify(actual) === JSON.stringify(expected)) passed += 1
  else failures.push(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
}

// ── Instruments first ───────────────────────────────────────────────────────────────────────
const fontsCss = process.env.FONTS_CSS ?? ''
const haveFonts = fontsCss !== '' && existsSync(fontsCss)
if (process.env.REQUIRE_FONTS === '1') assert(haveFonts, `REQUIRE_FONTS=1 but FONTS_CSS is unset or missing: "${fontsCss}"`)
console.log(haveFonts ? `fonts: ${fontsCss}` : 'WARNING fonts: FONTS_CSS unset or missing, the PNGs use a fallback face (assertions unaffected)')
const axePath = process.env.AXE_JS ?? '/root/.bun/install/cache/axe-core@4.12.1@@@1/axe.min.js'
const haveAxe = existsSync(axePath)
if (process.env.REQUIRE_AXE === '1') assert(haveAxe, `REQUIRE_AXE=1 but no axe at ${axePath}`)
console.log(haveAxe ? `axe: ${axePath}` : 'WARNING axe: not found, accessibility violations are NOT checked')

// Bundle the production entry. bun writes entry.js and entry.css (design.css + macPopover.css).
const entry = path.join(output, 'entry.tsx')
await writeFile(entry, `import ${JSON.stringify(path.join(repo, 'src/main.tsx'))}\n`)
const built = spawnSync('bun', ['build', entry, '--outdir', path.join(output, 'bundle'), '--target', 'browser', '--format', 'iife'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)

let fontFaces = ''
if (haveFonts) {
  const dir = path.dirname(fontsCss)
  fontFaces = (await readFile(fontsCss, 'utf8')).replace(/url\((?!['"]?(?:file|data|https?):)['"]?([^)'"]+)['"]?\)/g, (_, rel) => `url(${pathToFileURL(path.join(dir, rel)).href})`)
}
// The CSS is inlined: axe reads stylesheets back, and a file: stylesheet is a cross-origin read it refuses.
const bundleCss = await readFile(path.join(output, 'bundle', 'entry.css'), 'utf8')
const html = (title) => `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>${title}</title>
<style>${bundleCss}</style><style>${fontFaces}</style>
<script>Date.now = () => ${NOW * 1000}</script><script>${tauriMockInitScript()}</script></head>
<body><div id="root"></div><script src="bundle/entry.js"></script></body></html>`
await writeFile(path.join(output, 'index.html'), html('popover'))
const pageUrl = (query) => `${pathToFileURL(path.join(output, 'index.html')).href}${query}`
const POPOVER = '?window=popover&platform=macos'

// The page-side scripted backend. One scenario object, changed from the test between steps.
const BACKEND = () => {
  window.__scenario = { theme: 'light', snapshot: null, snapshotError: false, unlock: 'ok', installGate: null, installResult: null }
  window.__handler = async (cmd, args) => {
    const sc = window.__scenario
    if (cmd === 'desktop_config') return { theme: sc.theme }
    if (cmd === 'popover_snapshot') {
      if (sc.snapshotError) throw new Error('snapshot failed')
      return sc.snapshot
    }
    if (cmd === 'popover_unlock_vault') {
      if (sc.unlock === 'wrong') throw 'wrong_password'
      return null
    }
    if (cmd === 'install_finder_location') {
      if (sc.installGate) await sc.installGate
      return sc.installResult
    }
    return null
  }
}

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
const pageErrors = []
async function open(query = POPOVER, { dark = false, scenario = {}, track = true } = {}) {
  const context = await browser.newContext({ viewport: { width: 372, height: 488 }, deviceScaleFactor: 1, colorScheme: dark ? 'dark' : 'light', locale: 'nl-NL' })
  const page = await context.newPage()
  // Errors are only a verdict on the popover. The negative-mount cases load the OLD windows against a
  // bridge that knows nothing about them, which is allowed to complain.
  if (track) {
    page.on('pageerror', (e) => pageErrors.push(String(e)))
    page.on('console', (m) => { if (m.type() === 'error') pageErrors.push('console: ' + m.text()) })
  }
  await page.goto(pageUrl(query))
  await page.evaluate(BACKEND)
  await page.evaluate((sc) => Object.assign(window.__scenario, sc), scenario)
  return page
}
const show = async (page) => {
  await page.evaluate(() => window.__emit('popover-shown', null))
}
const stateOf = (page) => page.locator('.mp-root').getAttribute('data-state')
async function until(page, id) {
  await page.waitForFunction((want) => document.querySelector('.mp-root')?.getAttribute('data-state') === want, id, { timeout: 5000 }).catch(async () => {
    throw new Error(`state "${id}" never appeared; the popover says "${await stateOf(page).catch(() => 'nothing')}"`)
  })
  await page.evaluate(() => document.fonts.ready)
}

// What one rendered state must satisfy.
async function assertGeometry(page, id) {
  const geo = await page.evaluate(() => {
    const root = document.querySelector('.mp-root')
    const r = root.getBoundingClientRect()
    const rootBox = [r.x, r.y, r.width, r.height]
    const over = []
    const spill = []
    for (const el of [document.documentElement, document.body, root, ...root.querySelectorAll('*')]) {
      const cs = getComputedStyle(el)
      const scrolls = el === document.documentElement || el === document.body || cs.overflowX !== 'visible'
      // A deliberately truncated line (`text-overflow: ellipsis`) is allowed to be wider inside than out.
      if (scrolls && cs.textOverflow !== 'ellipsis' && el.scrollWidth > el.clientWidth) over.push(`${el.tagName.toLowerCase()}.${el.className}: ${el.scrollWidth} > ${el.clientWidth}`)
      const b = el.getBoundingClientRect()
      if (el !== document.documentElement && el !== document.body && b.width > 0 && (b.left < rootBox[0] - 0.5 || b.right > rootBox[0] + rootBox[2] + 0.5)) spill.push(`${el.tagName.toLowerCase()}.${el.className}: ${b.left}..${b.right}`)
    }
    const title = document.querySelector('[data-mp-title]')
    return { rootBox, over, spill, titleY: title ? title.getBoundingClientRect().y : null, titles: document.querySelectorAll('[data-mp-title]').length }
  })
  eq(geo.rootBox, [0, 0, 372, 488], `${id}: popover box`)
  eq(geo.over, [], `${id}: horizontal overflow (scrollWidth > clientWidth)`)
  eq(geo.spill, [], `${id}: elements outside the popover's left/right edges`)
  return geo
}
async function assertAxe(page, id) {
  if (!haveAxe) return
  await page.addScriptTag({ path: axePath })
  const results = await page.evaluate(async () => {
    // color-contrast is checked against the real tokens; the mock window has no wallpaper behind it.
    const r = await window.axe.run(document.querySelector('.mp-root'), { resultTypes: ['violations'] })
    return r.violations.map((v) => `${v.id}: ${v.nodes.length} node(s), ${v.nodes[0].html.slice(0, 90)}`)
  })
  eq(results, [], `${id}: axe violations`)
}
async function shoot(page, file) {
  await page.addStyleTag({ content: '*{animation:none!important}' })
  await page.locator('.mp-root').screenshot({ path: path.join(output, 'png', `${file}.png`), omitBackground: true })
}

// Put the design's render beside ours. The design PNGs are scenes (menu bar, wallpaper); the
// popover sits at (144, 30), 372 x 488. The numeric diff covers the interior only (14 px inset),
// so the wallpaper, the window shadow and the rounded corners do not count.
async function compare(designName, ownName) {
  if (!designDir) return null
  const designPng = path.join(designDir, `${designName}.png`)
  if (!existsSync(designPng)) return null
  const cmpPage = await (await browser.newContext({ viewport: { width: 1200, height: 560 }, deviceScaleFactor: 1 })).newPage()
  // data: URLs, not file: URLs, so the canvas is not tainted and the pixels can be read back.
  const dataUrl = async (file) => `data:image/png;base64,${(await readFile(file)).toString('base64')}`
  const ours = await dataUrl(path.join(output, 'png', `${ownName}.png`))
  const theirs = await dataUrl(designPng)
  await writeFile(path.join(output, 'compare', `${ownName}.html`), `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#8a93a6;font:12px/1.4 system-ui;color:#fff">
<div style="display:flex;gap:24px;padding:16px;align-items:flex-start">
<div><div>design (${designName}.png, cropped to the popover)</div><div id="d" style="width:372px;height:488px;background:url(${theirs}) -144px -30px no-repeat;border-radius:12px"></div></div>
<div><div>implementation (${ownName}.png)</div><img id="o" src="${ours}" width="372" height="488"></div>
<div><div>difference (interior, amplified x4)</div><canvas id="x" width="372" height="488"></canvas></div></div>
<script>
const load = (src) => new Promise((res) => { const i = new Image(); i.onload = () => res(i); i.src = src })
Promise.all([load(${JSON.stringify(theirs)}), load(${JSON.stringify(ours)})]).then(([d, o]) => {
  const mk = (img, sx, sy) => { const c = document.createElement('canvas'); c.width = 372; c.height = 488; const g = c.getContext('2d'); g.fillStyle = '#fff'; g.fillRect(0,0,372,488); g.drawImage(img, sx, sy, 372, 488, 0, 0, 372, 488); return g.getImageData(0, 0, 372, 488).data }
  const a = mk(d, 144, 30), b = mk(o, 0, 0)
  const out = document.getElementById('x').getContext('2d'); const img = out.createImageData(372, 488)
  let sum = 0, n = 0, over = 0
  for (let y = 0; y < 488; y++) for (let x = 0; x < 372; x++) {
    const i = (y * 372 + x) * 4
    const dd = (Math.abs(a[i] - b[i]) + Math.abs(a[i+1] - b[i+1]) + Math.abs(a[i+2] - b[i+2])) / 3
    img.data[i] = img.data[i+1] = img.data[i+2] = 255 - Math.min(255, dd * 4); img.data[i+3] = 255
    if (x >= 14 && x < 358 && y >= 14 && y < 474) { sum += dd; n++; if (dd > 24) over++ }
  }
  out.putImageData(img, 0, 0)
  window.__diff = { meanAbs: sum / n, pctOver24: 100 * over / n }
  document.title = 'done'
})
</script>`)
  await cmpPage.goto(pathToFileURL(path.join(output, 'compare', `${ownName}.html`)).href)
  await cmpPage.waitForFunction(() => document.title === 'done')
  const diff = await cmpPage.evaluate(() => window.__diff)
  await cmpPage.screenshot({ path: path.join(output, 'compare', `${ownName}.png`) })
  await cmpPage.context().close()
  return diff
}

const diffs = {}
const titleYs = {}
const OPEN_LIST = ['synced', 'conflict', 'syncing', 'syncing0']

// ── 1. Every state ──────────────────────────────────────────────────────────────────────────
const CASES = [
  // [state id, own file, design render, dark]
  ['synced', 'a-up-to-date', 'a-up-to-date'],
  ['conflict', 'a1-conflict', 'a1-conflict'],
  ['empty', 'a2-up-to-date-empty', 'a2-up-to-date-empty'],
  ['syncing', 'b-syncing', 'b-syncing'],
  ['syncing0', 'b2-syncing-first-build', 'b2-syncing-first-build'],
  ['locked', 'c1-vault-locked', 'c1-vault-locked'],
  ['unlock', 'c1b-unlock', 'c1b-unlock'],
  ['paused', 'c2-paused', 'c2-paused'],
  ['error', 'd-error', 'd-error'],
  ['offline', 'd2-offline', 'd2-offline'],
  ['storage', 's-storage-full', 's-storage-full'],
  ['signedout', 'e-signed-out', 'e-signed-out'],
  ['ended', 'e2-session-ended', 'e2-session-ended'],
  ['finder', 'f-finder-missing', 'f-finder-missing'],
  ['finderadding', 'f1-finder-adding', 'f1-finder-adding'],
  ['finderfail', 'f2-finder-failed', 'f2-finder-failed'],
  ['finderoff', 'f3-finder-turned-off', null],
  ['loadfail', 'z-load-failed', null],
  ['synced', 'g-dark-up-to-date', 'g-dark-up-to-date', true],
  ['error', 'h-dark-error', 'h-dark-error', true],
]
for (const [id, file, designName, dark] of CASES) {
  const start = id === 'unlock' ? 'locked' : id === 'finderadding' ? 'finder' : id
  const scenario = { theme: dark ? 'dark' : 'light', snapshot: id === 'loadfail' ? null : SNAPSHOTS[start], snapshotError: id === 'loadfail' }
  const page = await open(POPOVER, { dark, scenario })
  // The instrument: before anything is shown the popover is the blank first frame and has made no call.
  eq([await stateOf(page), await page.evaluate(() => window.__appCalls().length)], ['loading', 0], `${file}: first frame, no call before popover-shown`)
  await show(page)
  if (dark) await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark')
  if (id === 'unlock') {
    await until(page, 'locked')
    await page.getByRole('button', { name: 'Unlock vault' }).click()
    await until(page, 'unlock')
    await page.evaluate(() => { window.__scenario.unlock = 'wrong' })
    await page.getByLabel('Vault password').fill('not-my-password')
    await page.keyboard.press('Enter')
    await page.getByText('That password didn’t work. Try again.').waitFor()
  } else if (id === 'finderadding') {
    await until(page, 'finder')
    await page.evaluate(() => { window.__scenario.installGate = new Promise((r) => { window.__releaseInstall = r }); window.__scenario.installResult = { installed: false } })
    await page.getByRole('button', { name: 'Add to Finder' }).click()
  } else {
    await until(page, id)
  }
  await until(page, id)
  const geo = await assertGeometry(page, file)
  await assertAxe(page, file)
  if (geo.titles === 1) titleYs[file] = geo.titleY
  await shoot(page, file)
  if (designName) diffs[file] = await compare(designName, file)
  await page.context().close()
}
const ys = Object.entries(titleYs)
check(ys.length >= 14, `message states measured: ${ys.length}`)
eq([...new Set(ys.map(([, y]) => y))].length, 1, `title y identical across ${ys.length} message states (${JSON.stringify(titleYs)})`)

// ── 2. Stress: long content must truncate, never spill ────────────────────────────────────────
{
  const long = 'a-very-long-name-that-just-keeps-going-and-going-and-going-until-it-is-far-too-wide-for-any-popover'
  const stress = JSON.parse(JSON.stringify(SNAPSHOTS.synced))
  stress.account.email = `${long}@example-with-a-long-domain-name.eu`
  stress.storage = { used_bytes: 999.94e12, quota_bytes: 999.99e12, fetched_at: NOW, stale: false }
  stress.activity = SNAPSHOTS.synced.activity.map((r, i) => ({ ...r, name: `${long}-${i}.xlsx`, folder: long }))
  stress.conflicts = { count: 1, files: [{ file_id: 'f', file_name: `${long}.xlsx` }] }
  const stressErr = JSON.parse(JSON.stringify(SNAPSHOTS.error))
  stressErr.account.email = stress.account.email
  stressErr.reason = { code: 'x', detail: 'a-reason-line-that-is-much-longer-than-thirty-characters' }
  const stressFinder = JSON.parse(JSON.stringify(SNAPSHOTS.finderfail))
  stressFinder.finder.reason_line = 'reason: a-category-that-is-far-longer-than-thirty-characters'
  for (const [name, snap] of [['stress-synced', stress], ['stress-error', stressErr], ['stress-finderfail', stressFinder]]) {
    const page = await open(POPOVER, { scenario: { snapshot: snap } })
    await show(page)
    await until(page, snap.phase === 'synced' ? 'conflict' : snap.phase === 'error' ? 'error' : 'finderfail')
    await assertGeometry(page, name)
    await shoot(page, name)
    if (snap.phase === 'error' || snap.phase === 'finder_failed') {
      const detail = await page.evaluate(() => [...document.querySelectorAll('[title]')].map((e) => e.textContent).find((t) => t && t.length >= 20 && /…$/.test(t)))
      check(detail !== undefined && detail.length <= 30, `${name}: the mono line is cut to 30 characters (${detail})`)
    }
    await page.context().close()
  }
}

// ── 3. Mounted only at its own URL ────────────────────────────────────────────────────────────
for (const [query, expectPopover] of [[POPOVER, true], ['?window=popover&platform=windows', false], ['?window=popover', false], ['', false]]) {
  const page = await open(query, { track: expectPopover })
  await page.waitForTimeout(300)
  eq(await page.locator('.mp-root').count(), expectPopover ? 1 : 0, `${query || '(no query)'}: popover mounted = ${expectPopover}`)
  await page.context().close()
}

// ── 4. Silence while hidden; events while visible ─────────────────────────────────────────────
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.synced } })
  await page.waitForFunction(() => window.__listenerCount('popover-shown') === 1 && window.__listenerCount('engine-status') === 1 && window.__listenerCount('tauri://blur') === 1)
  eq(await page.evaluate(() => [window.__listenerCount('popover-shown'), window.__listenerCount('engine-status'), window.__listenerCount('tauri://blur')]), [1, 1, 1], 'listeners registered once (popover-shown, engine-status, blur)')
  await page.evaluate(() => { for (let i = 0; i < 10; i++) window.__emit('engine-status', { state: 'syncing' }) })
  await page.waitForTimeout(400)
  eq(await page.evaluate(() => window.__appCalls().length), 0, 'hidden, never shown: 10 engine-status events -> 0 app calls')
  await show(page)
  await until(page, 'synced')
  eq(await page.evaluate(() => window.__appCalls().map((c) => c.cmd).sort()), ['desktop_config', 'popover_snapshot'], 'shown: one theme read and one snapshot')
  await page.evaluate(() => window.__emit('engine-status', { state: 'syncing' }))
  await page.waitForFunction(() => window.__appCalls().filter((c) => c.cmd === 'popover_snapshot').length === 2)
  eq(await page.evaluate(() => window.__appCalls().filter((c) => c.cmd === 'popover_snapshot').length), 2, 'visible: an engine-status event refreshes the snapshot once')
  await page.evaluate(() => { window.__emit('tauri://blur', null) })
  const before = await page.evaluate(() => window.__appCalls().length)
  await page.evaluate(() => { for (let i = 0; i < 10; i++) window.__emit('engine-status', { state: 'syncing' }) })
  await page.waitForTimeout(400)
  eq(await page.evaluate(() => window.__appCalls().length), before, 'blurred: 10 engine-status events -> 0 new app calls')
  await show(page)
  await page.waitForFunction((n) => window.__appCalls().length > n, before)
  eq(await page.evaluate(() => window.__appCalls().filter((c) => c.cmd === 'popover_snapshot').length), 3, 'shown again: one more snapshot')
  await page.context().close()
}

// ── 5. Keyboard: focus goes to the first control, Tab order, trap, Esc ────────────────────────
async function focusOrder(page, steps) {
  const seen = []
  for (let i = 0; i < steps; i++) {
    seen.push(await page.evaluate(() => {
      const a = document.activeElement
      return a && a !== document.body ? (a.getAttribute('aria-label') || a.textContent || a.tagName).trim().slice(0, 40) : '(none)'
    }))
    await page.keyboard.press('Tab')
  }
  return seen
}
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.paused } })
  await show(page)
  await until(page, 'paused')
  eq(await focusOrder(page, 7), ['Beebeeb menu', 'Resume sync', 'Open in Finder', 'View online', 'Add storage', 'Beebeeb menu', 'Resume sync'], 'message state: focus starts on the gear; Tab = gear, primary action, 3 footer actions, then wraps (trapped)')
  await page.context().close()
}
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.paused } })
  await show(page)
  await until(page, 'paused')
  eq(await page.evaluate(() => document.activeElement.getAttribute('aria-label')), 'Beebeeb menu', 'on show, focus is on the first control (the gear)')
  const ring = () => page.evaluate(() => { const cs = getComputedStyle(document.activeElement); return [cs.outlineStyle, cs.outlineWidth] })
  eq(await ring(), ['none', '0px'], 'opened with the pointer: the focused gear is not ringed')
  await page.keyboard.press('Shift+Tab')
  eq(await page.evaluate(() => document.activeElement.textContent.trim()), 'Add storage', 'Shift+Tab from the gear wraps to the last footer action')
  eq(await ring(), ['solid', '2px'], 'after the first Tab the focus ring is a 2px solid ring')
  eq(await page.evaluate(() => getComputedStyle(document.activeElement).outlineColor === getComputedStyle(document.querySelector('.mp-root')).color), true, 'the ring is --ink, not amber')
  await page.context().close()
}
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.conflict } })
  await show(page)
  await until(page, 'conflict')
  eq(await focusOrder(page, 6), ['Beebeeb menu', 'Recent activity', 'Review', 'Open in Finder', 'View online', 'Add storage'], 'list state: gear, the list (one tab stop), Review, 3 footer actions')
  await page.locator('.mp-list').focus()
  const top0 = await page.evaluate(() => document.querySelector('.mp-list').scrollTop)
  await page.keyboard.press('ArrowDown')
  await page.waitForTimeout(150)
  const top1 = await page.evaluate(() => document.querySelector('.mp-list').scrollTop)
  check(top1 > top0, `ArrowDown moves the list (scrollTop ${top0} -> ${top1})`)
  await page.context().close()
}
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.synced } })
  await show(page)
  await until(page, 'synced')
  const before = await page.evaluate(() => window.__appCalls().length)
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'plugin:window|hide'))
  eq(await page.evaluate(() => window.__calls.filter((c) => c.cmd === 'plugin:window|hide').length), 1, 'Esc hides the window once')
  eq(await page.evaluate(() => window.__appCalls().length), before, 'Esc makes no app command')
  await page.evaluate(() => { for (let i = 0; i < 5; i++) window.__emit('engine-status', { state: 'syncing' }) })
  await page.waitForTimeout(300)
  eq(await page.evaluate(() => window.__appCalls().length), before, 'after Esc the popover is hidden: engine-status makes no call')
  await page.context().close()
}
{
  // Esc also works from inside the password field (it is handled on the window).
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.locked } })
  await show(page)
  await until(page, 'locked')
  await page.getByRole('button', { name: 'Unlock vault' }).click()
  await until(page, 'unlock')
  eq(await page.evaluate(() => document.activeElement.getAttribute('aria-label')), 'Vault password', 'the unlock form focuses its password field')
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'plugin:window|hide'))
  check(true, 'Esc from the password field hides')
  await page.context().close()
}

// ── 6. Actions call what they say (counted) ───────────────────────────────────────────────────
async function pressAndRead(snapshotKey, button, wait) {
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS[snapshotKey] } })
  await show(page)
  await until(page, snapshotKey)
  const before = await page.evaluate(() => window.__calls.length)
  await (button.startsWith('.') ? page.locator(button) : page.getByRole('button', { name: button, exact: true })).click()
  await page.waitForFunction((n) => window.__calls.length > n, before)
  await page.waitForTimeout(wait ?? 150)
  const calls = await page.evaluate((n) => window.__calls.slice(n).filter((c) => !c.cmd.startsWith('plugin:event')), before)
  await page.context().close()
  return calls
}
eq((await pressAndRead('paused', 'Resume sync')).map((c) => c.cmd), ['tray_resume_sync', 'popover_snapshot'], 'Resume sync -> tray_resume_sync, then a refresh')
eq((await pressAndRead('synced', 'Open in Finder')).map((c) => [c.cmd, c.args]), [['open_finder_location', { path: null }], ['plugin:window|hide', { label: 'popover' }]], 'Open in Finder -> open_finder_location(null), then hide')
eq((await pressAndRead('synced', 'Add storage')).map((c) => c.cmd), ['plugin:opener|open_url', 'plugin:window|hide'], 'Add storage -> opens the billing URL, then hide')
eq((await pressAndRead('storage', '.mp-primary')).filter((c) => c.cmd === 'plugin:opener|open_url').map((c) => c.args.url), ['https://app.beebeeb.io/billing'], 'storage full: the primary Add storage opens the billing URL (1 call)')
eq((await pressAndRead('signedout', 'Set up Beebeeb')).map((c) => c.cmd), ['open_onboarding_window', 'plugin:window|hide'], 'Set up Beebeeb -> open_onboarding_window, then hide')
eq((await pressAndRead('ended', 'Sign in')).map((c) => c.cmd), ['open_onboarding_window', 'plugin:window|hide'], 'Sign in -> open_onboarding_window, then hide')
eq((await pressAndRead('finderoff', 'Open System Settings')).map((c) => c.cmd), ['open_login_items_and_extensions_settings', 'plugin:window|hide'], 'Open System Settings -> open_login_items_and_extensions_settings, then hide')
eq((await pressAndRead('conflict', 'Review')).map((c) => [c.cmd, c.args]), [['open_conflict_window', { fileId: 'f1', fileName: 'Budget 2027.xlsx', isText: false }], ['plugin:window|hide', { label: 'popover' }]], 'Review -> open_conflict_window for the first file, then hide')
{
  const calls = await pressAndRead('synced', 'Beebeeb menu')
  eq(calls.map((c) => c.cmd), ['popover_gear_menu'], 'the gear asks for the native menu and does NOT hide the popover')
  check(Number.isInteger(calls[0].args.x) && Number.isInteger(calls[0].args.y) && calls[0].args.x > 300 && calls[0].args.y > 30 && calls[0].args.y < 80, `the gear menu is anchored at the gear's corner (${JSON.stringify(calls[0].args)})`)
}
{
  // Add to Finder: f1 at once, the command runs once, then the refreshed snapshot decides.
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.finder } })
  await show(page)
  await until(page, 'finder')
  await page.evaluate(() => { window.__scenario.installGate = new Promise((r) => { window.__releaseInstall = r }); window.__scenario.installResult = { installed: false, status: 'error' } })
  await page.getByRole('button', { name: 'Add to Finder' }).click()
  await until(page, 'finderadding')
  eq(await page.evaluate(() => document.querySelector('.mp-footer button').disabled), true, 'f1: Open in Finder is disabled')
  eq(await page.getByRole('button', { name: 'Adding…' }).isDisabled(), true, 'f1: the primary action reads Adding… and is disabled')
  await page.evaluate(() => { window.__scenario.snapshot = { ...window.__scenario.snapshot, phase: 'finder_failed', finder: { setup: 'failed', reason: 'timeout', reason_line: 'reason: timeout' } }; window.__releaseInstall() })
  await until(page, 'finderfail')
  eq(await page.evaluate(() => window.__appCalls().filter((c) => c.cmd === 'install_finder_location').length), 1, 'Add to Finder ran install_finder_location once')
  eq(await page.getByText('reason: timeout').count(), 1, 'f2 names the failure once (one mono line)')
  eq(await page.locator('[role="alert"]:not(#mp-unlock-line)').count(), 0, 'f2: no second error surface (no notice strip, no toast)')
  await page.context().close()
}
{
  // The unlock command does not exist in Rust yet: the button must fail loudly, once, and stay.
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.locked } })
  await page.evaluate(() => { window.__handler = async (cmd) => { if (cmd === 'popover_snapshot') return window.__scenario.snapshot; if (cmd === 'desktop_config') return { theme: 'light' }; if (cmd === 'popover_unlock_vault') throw new Error('Command popover_unlock_vault not found'); return null } })
  await show(page)
  await until(page, 'locked')
  await page.getByRole('button', { name: 'Unlock vault' }).click()
  await page.getByLabel('Vault password').fill('x')
  await page.keyboard.press('Enter')
  await page.getByText('Couldn’t unlock the vault.').waitFor()
  eq(await page.getByText('Couldn’t unlock the vault.').count(), 1, 'a missing unlock command shows one notice, not the wrong-password line')
  eq(await page.evaluate(() => document.querySelector('[data-state]').getAttribute('data-state')), 'unlock', 'and the form stays')
  await shoot(page, 'z2-unlock-command-missing')
  await page.context().close()
}

// ── 7. Brand and honesty checks on the painted DOM ─────────────────────────────────────────────
{
  const page = await open(POPOVER, { scenario: { snapshot: SNAPSHOTS.synced } })
  await show(page)
  await until(page, 'synced')
  const facts = await page.evaluate(() => {
    const root = document.querySelector('.mp-root')
    const fonts = new Set([...root.querySelectorAll('*')].map((e) => getComputedStyle(e).fontFamily.split(',')[0].trim().replace(/['"]/g, '')))
    const emoji = /\p{Extended_Pictographic}/u.test(root.textContent)
    const nonButtons = [...root.querySelectorAll('[role="button"], [onclick]')].length
    return { fonts: [...fonts].sort(), emoji, nonButtons, label: root.getAttribute('aria-label'), role: root.getAttribute('role') }
  })
  check(facts.fonts.every((f) => f === 'Inter' || f === 'JetBrains Mono'), `only Inter and JetBrains Mono are used (${facts.fonts.join(', ')})`)
  eq(facts.emoji, false, 'no emoji in the popover text')
  eq(facts.nonButtons, 0, 'no div-buttons: every control is a real button')
  eq([facts.role, facts.label], ['dialog', 'Beebeeb'], 'role=dialog, aria-label=Beebeeb')
  await page.context().close()
}

await browser.close()
const bad = pageErrors.filter(Boolean)
if (bad.length) failures.push(`page errors: ${JSON.stringify(bad.slice(0, 5))}`)
await writeFile(path.join(output, 'diffs.json'), JSON.stringify(diffs, null, 1))
await writeFile(path.join(output, 'title-y.json'), JSON.stringify(titleYs, null, 1))
for (const f of failures) console.error('FAIL', f)
console.log(`mac popover: ${passed} passed, ${failures.length} failed${haveAxe ? '' : ' (axe NOT run)'}`)
for (const [k, v] of Object.entries(diffs)) if (v) console.log(`  diff ${k}: mean ${v.meanAbs.toFixed(2)}/255, ${v.pctOver24.toFixed(2)}% of interior pixels differ by more than 24`)
process.exit(failures.length ? 1 : 0)
