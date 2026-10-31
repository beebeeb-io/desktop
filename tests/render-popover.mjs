// Real-browser check of the menu-bar popover (task 1683 slice 3; spec docs/specs/2026-10-24-macos-two-surface-app.md
// §6; hi-fi design/hifi/macos-two-surface.html section 1). Local evidence, not a CI gate (Playwright is not a desktop
// dependency); tests/popoverModel.test.ts and tests/popoverApp.test.tsx pin the same rules in CI.
//
//   node tests/render-popover.mjs REPO OUTPUT
//   PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs   (as the other render-*.mjs)
//
// It bundles the real PopoverApp (tests/fixtures/popover-render-entry.tsx), answers `popover_snapshot` with each
// fixture of tests/fixtures/popover/ (activity times moved to "now"), and draws every state at 372 x 488 in light and
// dark. Checks per drawing: the document never scrolls; the popover is exactly 372 x 488; nothing but an ellipsised
// line is wider than its box; the state's words show; no page errors. Exits 1 on any failed check, else prints a count.
import assert from 'node:assert/strict'
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-popover.mjs REPO OUTPUT')
await mkdir(path.join(output, 'png'), { recursive: true })

const outdir = path.join(output, 'bundle')
const built = spawnSync('bun', ['build', path.join(repo, 'tests/fixtures/popover-render-entry.tsx'), '--outdir', outdir, '--target', 'browser', '--format', 'iife', '--define', 'process.env.NODE_ENV="development"'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)
const pagePath = path.join(output, 'page.html')
await writeFile(pagePath, '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><link rel="stylesheet" href="bundle/popover-render-entry.css"></head><body><div id="root"></div><script src="bundle/popover-render-entry.js"></script></body></html>')

/** The words each state must show (§6.3, copy verbatim; the model's tests pin the rest). */
const MUST = {
  synced: ['Recent activity', 'Up to date', 'Encrypted', 'Using 84,3 GB of 200 GB', '2 min ago'],
  syncing: ['14 files left', '1,2 GB · 38 %', 'Done', 'Syncing', 'Encrypting and uploading · 62 %'],
  'needs-decision': ['2 files need a decision', 'Both changed on two devices.', 'Review…', 'Changed on two devices'],
  'update-available': ['Version 0.8.14 is available.', 'Download update', 'Drag Beebeeb into Applications to replace this version.'],
  paused: ['Sync is paused', '3 changes on this Mac are waiting. Nothing syncs until you resume.', 'Resume sync'],
  offline: ['Can’t reach Beebeeb', 'Check your connection. Files on this Mac are unchanged.', 'Try again'],
  error: ['Sync stopped', 'reason: server 503', 'Try again'],
  locked: ['Your vault is locked', 'Unlock vault'],
  'storage-full': ['Your storage is full', 'Using 200 GB of 200 GB', 'Add storage'],
  'finder-adding': ['Adding Beebeeb to Finder…', 'Recent activity'],
  'finder-failed': ['Couldn’t add Beebeeb to Finder', 'macOS didn’t finish adding Beebeeb to Finder.', 'reason: timeout'],
  'finder-not-in-applications': ['Beebeeb is running from the disk image. Move it to Applications, then open it again.', 'Show in Finder'],
  'finder-user-disabled': ['Beebeeb is turned off in System Settings.', 'Open System Settings'],
  'finder-missing': ['Recent activity', 'Up to date'],
  'sign-out-sending': ['Sending changes first', 'Sending 3 changes before signing out…', 'This takes at most a minute.', 'Open Beebeeb'],
}
const MUST_NOT = { locked: ['Using '], offline: [], 'sign-out-sending': ['Recent activity'] }

const dir = path.join(repo, 'tests/fixtures/popover')
const names = (await readdir(dir)).filter((f) => f.endsWith('.json')).map((f) => f.replace(/\.json$/, '')).filter((n) => n in MUST)
assert.equal(names.length, Object.keys(MUST).length, 'every state in MUST has a fixture')

function installBackend(fx) {
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'popover' }, currentWebview: { label: 'popover' } },
    transformCallback: () => 1,
    unregisterCallback() {},
    invoke: async (name) => {
      if (name === 'popover_snapshot') return fx
      if (name === 'plugin:event|listen') return 1
      if (name === 'get_desktop_config') return { theme: 'system' }
      return null
    },
  }
}

function report() {
  const doc = document.documentElement
  const root = document.querySelector('.pop-root')
  const r = root.getBoundingClientRect()
  const wide = [...root.querySelectorAll('*')].filter((el) => {
    if (el.scrollWidth <= el.clientWidth + 1) return false
    const style = getComputedStyle(el)
    return style.textOverflow !== 'ellipsis' && style.overflow !== 'hidden'
  }).map((el) => el.className || el.tagName)
  return { scrolls: doc.scrollHeight > doc.clientHeight + 1 || doc.scrollWidth > doc.clientWidth + 1, w: r.width, h: r.height, wide, text: root.innerText }
}

const browser = await chromium.launch()
let checks = 0
const failures = []
try {
  for (const scheme of ['light', 'dark']) {
    const context = await browser.newContext({ viewport: { width: 372, height: 488 }, colorScheme: scheme, deviceScaleFactor: 2, locale: 'nl-NL' })
    for (const name of names) {
      const fx = JSON.parse(await readFile(path.join(dir, `${name}.json`), 'utf8'))
      const shift = Math.floor(Date.now() / 1000) - fx.generated_at
      fx.generated_at += shift
      for (const row of fx.activity) row.at += shift
      const page = await context.newPage()
      const errors = []
      page.on('pageerror', (e) => errors.push(String(e)))
      await page.addInitScript(installBackend, fx)
      await page.goto(pathToFileURL(pagePath).href)
      await page.waitForSelector('.pop-root .pop-email')
      await page.waitForTimeout(150)
      const r = await page.evaluate(report)
      const label = `${name}/${scheme}`
      const check = (ok, what) => { checks += 1; if (!ok) failures.push(`${label}: ${what}`) }
      check(!r.scrolls, 'the document scrolls')
      check(r.w === 372 && r.h === 488, `size ${r.w} x ${r.h}`)
      check(r.wide.length === 0, `wider than its box: ${r.wide.join(', ')}`)
      for (const words of MUST[name]) check(r.text.includes(words), `missing "${words}"`)
      for (const words of MUST_NOT[name] ?? []) check(!r.text.includes(words), `shows "${words}"`)
      check(errors.length === 0, `page errors: ${errors.join(' | ')}`)
      await page.screenshot({ path: path.join(output, 'png', `${name}-${scheme}.png`) })
      await page.close()
    }
    await context.close()
  }
  // One sheet per theme: every state side by side, for the comparison with the hi-fi's section 1.
  for (const scheme of ['light', 'dark']) {
    const sheet = path.join(output, `sheet-${scheme}.html`)
    const cells = names.map((n) => `<figure style="margin:0"><img width="372" height="488" src="${pathToFileURL(path.join(output, 'png', `${n}-${scheme}.png`)).href}"><figcaption style="font:12px system-ui">${n}</figcaption></figure>`).join('')
    await writeFile(sheet, `<!doctype html><body style="margin:16px;background:#888;display:grid;grid-template-columns:repeat(5,372px);gap:16px">${cells}</body>`)
    const page = await browser.newPage({ viewport: { width: 5 * 372 + 6 * 16, height: 800 } })
    await page.goto(pathToFileURL(sheet).href)
    await page.screenshot({ path: path.join(output, 'png', `sheet-${scheme}.png`), fullPage: true })
    await page.close()
  }
} finally {
  await browser.close()
}
if (failures.length) {
  console.error(`${failures.length} of ${checks} checks failed:\n${failures.join('\n')}`)
  process.exit(1)
}
console.log(`render-popover: ${checks} checks passed across ${names.length} states x 2 themes (${names.length * 2} drawings)`)
