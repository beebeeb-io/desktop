// Renders section 5 (Settings › Backup, LD-0113) of design/hifi/macos-settings-dialogs.html headlessly and checks it.
// Local design evidence, not a CI gate (Playwright is a dev dependency only).
//
//   node docs/specs/evidence/LD-0113/render-backup-panel.mjs [BASE_HTML]
//
// Viewport 1152 wide (the page's 1120 reading column plus its 16 px padding), device scale 2. Each state is shot as its
// window element: the Beebeeb window at its default 920 x 620 pt. Checks, each printed as a count:
//   1. the window is 920 x 620 (+1 px border each side), and its pane needs no scrolling at that height;
//   2. no element inside a window is wider than its box (scrollWidth <= clientWidth on every clipping element);
//   3. amber backgrounds per window: 0, 0, 0, 1 (only "Back up again", in the deleted state);
//   4. Inter and JetBrains Mono loaded (else the PNGs would show a fallback face);
//   5. with BASE_HTML (the file on origin/main): sections 1-4, Keyboard, Mapping, Copy and "not checked" render
//      pixel-identical before and after, so no existing drawing changed.
// Exits 1 on the first failed check.
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { chromium } from 'playwright'

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, '../../../..')
// DESIGN_HTML and OUT_DIR point a run at another copy (how the checks are proven red on a mutant) without touching
// the evidence PNGs.
const design = process.env.DESIGN_HTML ?? path.join(repo, 'design/hifi/macos-settings-dialogs.html')
const out = process.env.OUT_DIR ?? here
const base = process.argv[2]
await mkdir(out, { recursive: true })

const browser = await chromium.launch()
const open = async (file, theme) => {
  const page = await browser.newPage({ viewport: { width: 1152, height: 900 }, deviceScaleFactor: 2 })
  await page.goto(pathToFileURL(file).href, { waitUntil: 'networkidle' })
  if (theme) await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme)
  await page.evaluate(() => document.fonts.ready)
  return page
}

const page = await open(design)
const fonts = await page.evaluate(() => ({ inter: document.fonts.check('600 13px Inter'), mono: document.fonts.check('11.5px "JetBrains Mono"') }))
assert(fonts.inter && fonts.mono, `fonts not loaded: ${JSON.stringify(fonts)}`)
console.log('fonts loaded: Inter and JetBrains Mono')

const STATES = [
  ['bk-none', 'no-grants', 0],
  ['bk-running', 'granted-running', 0],
  ['bk-skips', 'skip-lines', 0],
  ['bk-deleted', 'deleted-elsewhere', 1],
]
let shots = 0
for (const [id, name, amberWanted] of STATES) {
  const m = await page.evaluate((id) => {
    const win = document.getElementById(id)
    const pane = win.querySelector('.ms-pane')
    const probe = document.createElement('i')
    probe.style.background = 'var(--amber)'
    win.appendChild(probe)
    const amber = getComputedStyle(probe).backgroundColor
    probe.remove()
    const amberCount = [...win.querySelectorAll('*')].filter((el) => getComputedStyle(el).backgroundColor === amber).length
    const wide = [...win.querySelectorAll('*')].filter((el) => {
      const s = getComputedStyle(el)
      const clips = s.overflowX !== 'visible' && s.textOverflow !== 'ellipsis'
      return clips && el.scrollWidth > el.clientWidth + 0.5
    }).map((el) => el.className)
    const r = win.getBoundingClientRect()
    return { w: r.width, h: r.height, paneScroll: pane.scrollHeight, paneClient: pane.clientHeight, amberCount, wide, rows: win.querySelectorAll('.ms-switch').length }
  }, id)
  assert.equal(m.w, 922, `${id} width`)
  assert.equal(m.h, 622, `${id} height`)
  assert(m.paneScroll <= m.paneClient, `${id}: the pane scrolls (${m.paneScroll} > ${m.paneClient})`)
  assert.deepEqual(m.wide, [], `${id}: overflowing elements`)
  assert.equal(m.amberCount, amberWanted, `${id}: amber backgrounds`)
  assert.equal(m.rows, 6, `${id}: folder switches`)
  const file = path.join(out, `macos-backup-${name}.png`)
  await page.locator(`#${id}`).screenshot({ path: file })
  shots++
  console.log(`${id}: ${m.w}x${m.h}, pane ${m.paneScroll}/${m.paneClient} px (fits), 0 overflowing, ${m.amberCount} amber, ${m.rows} switches -> ${path.relative(repo, file)}`)
}

const dark = await open(design, 'dark')
const darkFile = path.join(out, 'macos-backup-deleted-elsewhere-dark.png')
await dark.locator('#bk-deleted').screenshot({ path: darkFile })
shots++
console.log(`bk-deleted (dark) -> ${path.relative(repo, darkFile)}`)

if (base) {
  const before = await open(base)
  const SECTIONS = ['s-verdict', 's-signout', 's-repair', 's-folders', 's-switch', 's-kbd', 's-modal', 's-copy', 's-not']
  const sizes = (p) => p.evaluate((ids) => ids.map((id) => {
    const r = document.querySelector(`section[aria-labelledby="${id}"]`).getBoundingClientRect()
    return `${id} ${r.width}x${r.height}`
  }), SECTIONS)
  // With section 5 in place, every existing section keeps its exact size (sections below it only move down).
  assert.deepEqual(await sizes(page), await sizes(before), 'an existing section changed size')
  console.log(`existing sections the same size with section 5 in place: ${SECTIONS.length} of ${SECTIONS.length}`)
  // Pixels are compared with section 5's markup removed and its CSS still loaded: a section that moved down by a
  // fractional offset rasterises its text at another subpixel phase, which is the instrument, not a change. Both pages
  // start each shot scrolled to the top, so Playwright scrolls them the same way.
  await page.evaluate(() => document.querySelector('section.bk').remove())
  let same = 0
  for (const id of SECTIONS) {
    const sel = `section[aria-labelledby="${id}"]`
    await Promise.all([before, page].map((p) => p.evaluate(() => window.scrollTo(0, 0))))
    const [a, b] = await Promise.all([before.locator(sel).screenshot(), page.locator(sel).screenshot()])
    assert(a.equals(b), `section ${id} renders differently from the base file`)
    same++
  }
  console.log(`existing sections pixel-identical to the base file (section 5's CSS loaded, its markup removed): ${same} of ${SECTIONS.length}`)
}
await browser.close()
console.log(`${shots} screenshots, all checks passed`)
