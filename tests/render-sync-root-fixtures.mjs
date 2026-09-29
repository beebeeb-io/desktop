// Actual React/browser render + click checks with synthetic Tauri IPC.
// Native sync-root registration and Explorer destination remain NATIVE.md rungs.
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-sync-root-fixtures.mjs REPO OUTPUT')
const bundle = await readFile(path.join(output, 'sync-root-render.js'), 'utf8')
const css = await readFile(path.join(repo, 'src/design.css'), 'utf8')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))['windows-nsis']
await mkdir(output, { recursive: true })
const root = 'D:\\Private files\\資料\\Sync'
const defaultRoot = 'C:\\Users\\Fixture\\Beebeeb'
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let passed = 0
try {
  for (const scenario of ['loading', 'empty', 'populated', 'overview-error', 'unconfigured', 'settings', 'tray']) {
    const page = await browser.newPage({ viewport: scenario === 'tray' ? { width: 360, height: 520 } : { width: 1200, height: 900 } })
    const errors = []
    page.on('pageerror', error => errors.push(String(error)))
    await page.route('http://fixture.local/**', route => route.fulfill({ contentType: 'text/html', body: '<div id="root"></div>' }))
    await page.goto(`http://fixture.local/?nav=files&surface=${scenario === 'tray' ? 'tray' : 'main'}`)
    await page.addStyleTag({ content: css })
    await page.evaluate(({ caps, root, defaultRoot, scenario }) => {
      window.calls = []
      window.fixtureRoot = scenario === 'unconfigured' ? null : root
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: scenario === 'tray' ? 'tray' : 'main' }, currentWebview: { label: 'main' } },
        transformCallback: () => 1, unregisterCallback() {},
        invoke: async (name, args) => {
          window.calls.push({ name, args })
          if (name === 'desktop_capabilities') return caps
          if (name === 'sync_status') return { logged_in: true, engine: 'running', sync_root: window.fixtureRoot, syncing: 0, cloud_only: 0, conflicts: 0 }
          if (name === 'default_sync_root') return defaultRoot
          if (name === 'open_finder_location') return null
          if (name === 'get_known_folder_onboarding_seen') return true
          if (name === 'get_known_folder_backup') return []
          if (name === 'desktop_file_overview') {
            if (scenario === 'loading') return new Promise(() => {})
            if (scenario === 'overview-error') throw new Error('fixture overview failure')
            return { total_files: scenario === 'populated' ? 2 : 0, total_bytes: scenario === 'populated' ? 10 : 0, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
          }
          if (name === 'desktop_config') return { theme: 'system', local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0 }
          if (name === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 100, cache_bytes: 0, pinned_bytes: 0 }
          if (name === 'account_usage') return { storage_used_bytes: 0, storage_quota_bytes: 100 }
          if (name === 'account_region') return { region: null }
          if (name === 'windows_shell_integration_state') return { installed: true, path: defaultRoot }
          if (name.startsWith('plugin:')) return 1
          throw new Error(`Unhandled fixture command ${name}`)
        },
      }
    }, { caps, root, defaultRoot, scenario })
    await page.addScriptTag({ content: bundle })
    if (scenario === 'settings') {
      await page.getByRole('button', { name: 'Settings', exact: true }).click()
      await page.getByRole('button', { name: 'Explorer integration', exact: true }).click()
    }
    const expected = scenario === 'unconfigured' ? 'Not configured on this PC yet' : root
    await page.getByText(expected, { exact: true }).waitFor()
    assert.equal(await page.getByText(expected, { exact: true }).count(), 1)
    assert.equal(await page.getByText(defaultRoot, { exact: true }).count(), 0)
    if (scenario === 'overview-error') await page.getByText('fixture overview failure', { exact: true }).waitFor()
    if (scenario === 'empty') await page.getByText('No files on this PC yet', { exact: true }).waitFor()
    if (scenario !== 'settings') {
      const button = page.getByRole('button', { name: scenario === 'tray' ? 'Open folder' : 'Open in Explorer', exact: true })
      assert.equal(await button.isDisabled(), scenario === 'unconfigured')
      if (scenario !== 'unconfigured') {
        await button.click()
        await page.waitForFunction(() => window.calls.some(c => c.name === 'open_finder_location'))
        assert.deepEqual(await page.evaluate(() => window.calls.filter(c => c.name === 'open_finder_location').map(c => c.args)), [{}])
      }
    }
    await page.screenshot({ path: path.join(output, `${scenario}.png`), fullPage: true })
    if (scenario !== 'unconfigured') {
      const changed = 'E:\\Changed while open\\A long folder name\\Another long folder name\\資料\\Sync'
      await page.evaluate(value => { window.fixtureRoot = value }, changed)
      await page.getByText(changed, { exact: true }).waitFor({ timeout: 10000 })
      assert.equal(await page.getByText(root, { exact: true }).count(), 0)
      assert.equal(await page.getByText(changed, { exact: true }).getAttribute('title'), changed)
      await page.screenshot({ path: path.join(output, `${scenario}-changed.png`), fullPage: true })
    }
    assert.equal(await page.evaluate(() => window.calls.filter(c => c.name === 'default_sync_root').length), 0)
    assert.deepEqual(errors, [])
    await writeFile(path.join(output, `${scenario}-calls.json`), JSON.stringify(await page.evaluate(() => window.calls), null, 2))
    passed++
    await page.close()
  }
  assert.equal(passed, 7)
  console.log(`browser fixture cases: ${passed} passed; 0 failed; native cases: 0`)
} finally { await browser.close() }
