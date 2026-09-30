// Real-browser check of the locale-aware size formatter (task 1683 slice 2).
// Bundles src/storageFormat.ts for the browser, loads it in Chromium under a Dutch and an
// English browser locale, and asserts what the formatter prints with NO locale argument
// (the default path the popover uses: navigator.languages[0]) and with an explicit one.
// Saves one PNG per locale next to the design render's header so a human can compare.
//
//   node tests/render-storage-format.mjs REPO OUTPUT [DESIGN_HEADER_PNG]
//   PLAYWRIGHT_MODULE=/opt/node22/lib/node_modules/playwright/index.mjs
//
// Local evidence, not a CI gate (Playwright is not a desktop dependency). Prints a count line.
import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output, designPng] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-storage-format.mjs REPO OUTPUT [DESIGN_HEADER_PNG]')
await mkdir(output, { recursive: true })

const entry = path.join(output, 'storage-format-entry.ts')
await writeFile(entry, `import * as m from ${JSON.stringify(path.join(repo, 'src/storageFormat.ts'))}\n;(window as unknown as { storageFormat: typeof m }).storageFormat = m\n`)
const bundlePath = path.join(output, 'storage-format.browser.js')
const built = spawnSync('bun', ['build', entry, '--outfile', bundlePath, '--target', 'browser', '--format', 'iife'], { encoding: 'utf8' })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)

const cases = [
  // [browser locale, expected default-path output for 84.3 GB, for 200 GB, for 1.2345 TB]
  ['nl-NL', '84,3 GB', '200 GB', '1,2 TB'],
  ['en-US', '84.3 GB', '200 GB', '1.2 TB'],
]
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let passed = 0
const check = (actual, expected, label) => {
  assert.equal(actual, expected, label)
  passed += 1
}
try {
  for (const [locale, gb843, gb200, tb12] of cases) {
    const context = await browser.newContext({ locale, viewport: { width: 860, height: 150 }, deviceScaleFactor: 2 })
    const page = await context.newPage()
    const errors = []
    page.on('pageerror', (e) => errors.push(String(e)))
    await page.setContent('<!doctype html><meta charset="utf-8"><body></body>')
    await page.addScriptTag({ path: bundlePath })
    // The instrument first: the browser really is in this locale.
    const language = await page.evaluate(() => navigator.languages[0])
    check(language, locale, `${locale}: navigator.languages[0]`)
    const got = await page.evaluate(() => {
      const f = window.storageFormat
      return {
        defaultPath: [f.formatStorageSize(84_300_000_000), f.formatStorageSize(200_000_000_000), f.formatStorageSize(1_234_567_890_000)],
        explicitNl: f.formatStorageSize(84_300_000_000, 'nl'),
        explicitEn: f.formatStorageSize(84_300_000_000, 'en'),
        roundUp: f.formatStorageSize(999_960_000, 'en'),
        unknown: f.formatStorageSize(NaN),
      }
    })
    check(got.defaultPath[0], gb843, `${locale}: 84.3 GB on the default path`)
    check(got.defaultPath[1], gb200, `${locale}: 200 GB has no trailing decimal`)
    check(got.defaultPath[2], tb12, `${locale}: 1.2345 TB keeps one decimal`)
    check(got.explicitNl, '84,3 GB', `${locale}: explicit nl`)
    check(got.explicitEn, '84.3 GB', `${locale}: explicit en`)
    check(got.roundUp, '1 GB', `${locale}: 999.96 MB rounds up into GB`)
    check(got.unknown, '—', `${locale}: NaN is the dash`)
    // The PNG: my line in the design's face, beside the design render's header.
    const design = designPng ? pathToFileURL(designPng).href : ''
    // A file: page, because about:blank may not load file: images and fonts.
    const htmlPath = path.join(output, `compare-${locale}.html`)
    await writeFile(htmlPath, `<!doctype html><meta charset="utf-8">
      <link rel="stylesheet" href="${pathToFileURL('/home/user/evidence/1683/fonts.css').href}">
      <body style="margin:0;background:#f4f1ea;font-family:Inter,system-ui,sans-serif;display:flex;gap:24px;padding:16px;align-items:flex-start">
        <div><div style="font:11px monospace;color:#555;margin-bottom:6px">design render (a-up-to-date), header</div>
          ${design ? `<div style="width:380px;height:72px;overflow:hidden;border:1px solid #ccc;background:url(${design}) -144px -28px no-repeat"></div>` : '<i>no design png given</i>'}</div>
        <div><div style="font:11px monospace;color:#555;margin-bottom:6px">formatStorageSize, browser locale ${locale}</div>
          <div id="mine" style="width:380px;height:72px;border:1px solid #ccc;padding:14px 16px;box-sizing:border-box;background:#fbfaf6"></div></div>
      </body>`)
    await page.goto(pathToFileURL(htmlPath).href)
    await page.addScriptTag({ path: bundlePath })
    await page.evaluate(() => {
      const f = window.storageFormat
      document.getElementById('mine').innerHTML =
        '<div style="font-size:13px;font-weight:600;line-height:18px">sam@example.eu</div><div style="font-size:11.5px;color:#5b5b55;line-height:16px">Using ' +
        f.formatStorageSize(84_300_000_000) + ' of ' + f.formatStorageSize(200_000_000_000) + '</div>'
    })
    await page.evaluate(() => document.fonts.ready)
    await page.screenshot({ path: path.join(output, `storage-format-${locale}.png`) })
    assert.deepEqual(errors, [], `${locale}: page errors`)
    await context.close()
  }
} finally {
  await browser.close()
}
console.log(`${passed} passed, 0 failed (storage format in Chromium, 2 locales)`)
