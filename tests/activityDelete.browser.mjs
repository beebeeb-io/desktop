// Mount the production Activity view with deterministic IPC. This is not CFAPI QA.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href)
const [bundlePath, reportPath, screenshotPath] = process.argv.slice(2)
const bundle = await fs.readFile(bundlePath, 'utf8')
const css = await fs.readFile(new URL('../src/design.css', import.meta.url), 'utf8')
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
const results = []
try {
  for (const scenario of ['unresolved', 'http-failure', 'feed-failure']) {
    const page = await browser.newPage({ viewport: { width: 1100, height: 750 } })
    const errors = []
    page.on('pageerror', error => errors.push(String(error)))
    await page.setContent('<div id="root"></div>')
    await page.addStyleTag({ content: css })
    await page.evaluate(scenario => {
      window.__TAURI_INTERNALS__ = { invoke: async name => {
        if (name === 'account_region') return { preferred_region: null, regions: [] }
        if (name === 'account_activity_feed') return { events: [], total: 0 }
        if (name === 'list_version_conflict_center') {
          if (scenario === 'feed-failure') throw new Error('fixture database unavailable')
          return [{ id: 'op:delete', kind: 'delete', file_name: 'renamed.txt',
            detail: scenario === 'unresolved' ? 'Delete has no known identity; review required' : 'Server delete failed: HTTP 500; retry pending' }]
        }
        throw new Error('Unhandled fixture command ' + name)
      } }
    }, scenario)
    await page.addScriptTag({ content: bundle })
    await page.getByText('No activity yet', { exact: true }).waitFor()
    assert.equal(await page.getByText('Local sync issues', { exact: true }).count(), 1)
    if (scenario === 'feed-failure') {
      assert.equal(await page.getByRole('status').filter({ hasText: 'fixture database unavailable' }).count(), 1)
    } else {
      assert.equal(await page.getByText('renamed.txt', { exact: true }).count(), 1)
      assert.equal(await page.getByRole('status').filter({ hasText: scenario === 'unresolved' ? 'review required' : 'HTTP 500' }).count(), 1)
    }
    assert.deepEqual(errors, [])
    results.push({ scenario, passed: 1, text: await page.locator('body').innerText() })
    if (screenshotPath && scenario === 'unresolved') await page.screenshot({ path: screenshotPath })
    await page.close()
  }
  await fs.writeFile(reportPath, JSON.stringify({ passed: results.length, failed: 0, results }, null, 2))
  console.log(`Activity browser: ${results.length} passed; 0 failed`)
} finally {
  await browser.close()
}
