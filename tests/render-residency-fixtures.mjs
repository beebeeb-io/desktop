// Actual React/browser render with synthetic IPC; native OS gates are separate.
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-residency-fixtures.mjs REPO OUTPUT')
const bundle = await readFile(path.join(output, 'residency-render.js'), 'utf8')
const css = await readFile(path.join(repo, 'src/design.css'), 'utf8')
const capabilities = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))
await mkdir(output, { recursive: true })
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let passed = 0
try {
  for (const host of ['windows-nsis', 'macos']) {
    for (const regionState of ['eu', 'us', 'unknown', 'loading', 'error']) {
    for (const surface of ['sidebar', 'settings']) {
      const page = await browser.newPage({ viewport: { width: 1200, height: 900 } })
      page.setDefaultTimeout(5000)
      const errors = []
      page.on('pageerror', error => errors.push(String(error)))
      const route = surface === 'sidebar' ? (host === 'macos' ? 'onboarding' : 'main') : 'settings'
      const name = `${host}-${surface}-${regionState}`
      try {
        await page.route('http://fixture.local/**', r => r.fulfill({ contentType: 'text/html', body: '<div id="root"></div>' }))
        await page.goto(`http://fixture.local/?surface=${route}`)
        await page.addStyleTag({ content: css })
        await page.evaluate(({ caps, route, regionState }) => {
          window.calls = []
          window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
            transformCallback: () => 1, unregisterCallback() {},
            invoke: async (name, args) => {
              window.calls.push({ name, args })
              if (name === 'desktop_capabilities') return caps
              if (name === 'desktop_platform') return caps.host_os
              if (name === 'sync_status') return { logged_in: route !== 'onboarding', vault_unlocked: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }
              if (name === 'finder_location_state') return { installed: true }
              if (name === 'get_known_folder_onboarding_seen') return true
              if (name === 'get_known_folder_backup') return []
              if (name === 'desktop_file_overview') return { total_files: 2, total_bytes: 10, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
              if (name === 'desktop_config') return { theme: 'system', local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0 }
              if (name === 'desktop_storage_summary') return { used_bytes: 10, quota_bytes: 100, cache_bytes: 0, pinned_bytes: 0 }
              if (name === 'account_usage') return { used_bytes: 10, quota_bytes: 100, percentage: 0.1 }
              if (name === 'account_region') {
                if (regionState === 'loading') return new Promise(() => {})
                if (regionState === 'error') throw new Error('Fixture region request failed')
                return { preferred_region: regionState === 'us' ? 'us' : regionState === 'unknown' ? 'unknown' : null,
                  regions: [
                    { continent: 'europe', display_name: 'Europe', city: 'Falkenstein', is_default: true, provider: 'Never Display Storage Provider' },
                    ...(regionState === 'us' ? [{ continent: 'us', display_name: 'North America', city: 'Ashburn', is_default: false }] : []),
                  ] }
              }
              if (name.startsWith('plugin:')) return 1
              throw new Error(`Unhandled fixture command ${name}`)
            },
          }
        }, { caps: capabilities[host], route, regionState })
        await page.addScriptTag({ content: bundle })
        if (surface === 'settings') {
          await page.getByRole('button', { name: 'Data residency', exact: true }).click()
          const expected = regionState === 'eu' ? 'Stored in the EU (currently Falkenstein, Germany).'
            : regionState === 'us' ? 'Stored in North America (currently Ashburn, United States).'
            : regionState === 'loading' ? 'Loading data residency…' : 'Could not load data residency'
          await page.getByText(expected, { exact: true }).waitFor()
          if (regionState === 'eu') assert.equal(await page.getByRole('button', { name: /Europe/ }).isDisabled(), true)
          if (!['eu', 'us'].includes(regionState)) assert.doesNotMatch(await page.locator('body').innerText(), /Stored in|Falkenstein|Ashburn/)
          if (regionState === 'us') assert.doesNotMatch(await page.locator('body').innerText(), /Stored in the EU/)
          assert.equal(await page.evaluate(() => window.calls.filter(c => c.name === 'account_region').length), 1)
          assert.doesNotMatch(await page.locator('body').innerText(), /Choose where|More regions|Never Display Storage Provider/)
        } else {
          const label = regionState === 'eu' ? 'Stored in the EU' : regionState === 'us' ? 'Stored in North America' : 'End-to-end encrypted'
          await page.getByText(host === 'macos' ? `${label} | Zero-knowledge` : label, { exact: true }).waitFor()
          const body = await page.locator('body').innerText()
          if (!['eu', 'us'].includes(regionState)) assert.doesNotMatch(body, /Stored in|the EU|North America/)
          if (regionState === 'us') assert.doesNotMatch(body, /Stored in the EU/)
          assert.doesNotMatch(await page.locator('body').innerText(), /Falkenstein|Germany|Ashburn|Never Display Storage Provider/)
        }
        assert.deepEqual(errors, [])
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
        await page.screenshot({ path: path.join(output, `${name}.png`), fullPage: true })
        await writeFile(path.join(output, `${name}-calls.json`), JSON.stringify(await page.evaluate(() => window.calls), null, 2))
        passed++
      } catch (error) {
        await page.screenshot({ path: path.join(output, `${name}-failed.png`), fullPage: true })
        throw new Error(`${name}: ${error}`, { cause: error })
      } finally { await page.close() }
    }
  }
  }
  assert.equal(passed, 20)
  console.log(`browser fixture cases: ${passed} passed; 0 failed; screenshots: ${passed}; native cases: 0`)
} finally { await browser.close() }
