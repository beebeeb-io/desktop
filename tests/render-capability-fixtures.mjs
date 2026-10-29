// Fixture browser checks; not native backend/CFAPI proof. Run after bundling the
// tests/fixtures/capability-render.tsx entry as described in NATIVE.md.
import assert from 'node:assert/strict'
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-capability-fixtures.mjs REPO OUTPUT')
const fixtures = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))
const bundle = await readFile(path.join(output, 'capability-render.js'), 'utf8')
const css = await readFile(path.join(repo, 'src/design.css'), 'utf8')
await mkdir(output, { recursive: true })
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let cases = 0
try {
  async function pageFor(host, surface, options = {}) {
    const page = await browser.newPage({ viewport: options.viewport ?? { width: 1200, height: 900 }, colorScheme: options.dark ? 'dark' : 'light' })
    await page.setContent('<html><head></head><body><div id="root"></div></body></html>')
    await page.addStyleTag({ content: css })
    await page.evaluate(({ caps, surface, options }) => {
      window.capabilityFixture = { surface, route: options.route }
      window.calls = []
      window.__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (name, args) => {
          window.calls.push({ name, args })
          if (name === 'desktop_capabilities') {
            if (options.reject && window.calls.filter(c => c.name === name).length === 1) throw new Error('fixture capability failure')
            return caps
          }
          if (name === 'desktop_platform') throw new Error('snapshot must supply the host')
          if (name === 'desktop_config') return { theme: 'system', local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0 }
          if (name === 'desktop_storage_summary') return { used_bytes: 12, quota_bytes: 100, cache_bytes: 12, pinned_bytes: 0 }
          if (name === 'free_up_space') return { bytes_freed: 12 }
          if (name === 'account_region') return { region: null }
          if (name === 'app_activity_snapshot') return { pid: 1, process_name: 'fixture', cpu_percent: 0, memory_rss_bytes: 100 }
          if (name === 'autostart_enabled') return false
          if (name === 'finder_location_state' || name === 'windows_shell_integration_state') return { installed: false }
          if (name === 'install_finder_location' || name === 'install_windows_shell_integration') return { installed: true }
          if (name.startsWith('plugin:')) return 1
          throw new Error(`unhandled fixture command ${name}`)
        },
      }
    }, { caps: fixtures[host], surface, options })
    await page.addScriptTag({ content: bundle })
    return page
  }
  async function assertNotice(page) {
    const notice = page.locator('.capability-notice')
    await notice.waitFor()
    const styles = await notice.evaluate(el => {
      const body = getComputedStyle(el.querySelector('p'))
      const card = getComputedStyle(el.parentElement)
      return { fontSize: body.fontSize, lineHeight: body.lineHeight, margin: body.margin,
        color: body.color, headingColor: getComputedStyle(document.body).color,
        font: getComputedStyle(el).fontFamily, border: card.borderTopWidth, radius: card.borderRadius }
    })
    assert.equal(styles.fontSize, '12px')
    assert.equal(styles.lineHeight, '19.2px')
    assert.equal(styles.margin, '0px')
    assert.notEqual(styles.color, styles.headingColor)
    assert.match(styles.font, /Inter/)
    assert.equal(styles.border, '1px')
    assert.equal(styles.radius, '10px')
    assert.equal(await page.locator('.capability-notice button:not(.button)').count(), 0)
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  }
  for (const host of ['windows-nsis', 'windows-msi', 'macos']) {
    const page = await pageFor(host, 'settings')
    await page.getByText('Free up space', { exact: true }).last().waitFor()
    const windows = host.startsWith('windows')
    if (windows) await assertNotice(page)
    for (const name of ['Sync on metered connections', 'Show sync overlays in File Explorer', 'Files on Demand (online-only by default)', 'Keep files encrypted end-to-end']) {
      assert.equal(await page.getByRole('switch', { name, exact: true }).count(), windows ? 0 : 1)
    }
    await page.screenshot({ path: path.join(output, `${host}-sync.png`), fullPage: true })
    await page.getByRole('button', { name: 'Free up space', exact: true }).click()
    assert.equal(await page.evaluate(() => window.calls.filter(c => c.name === 'free_up_space').length), 1)
    await page.getByRole('button', { name: windows ? 'Explorer integration' : 'Finder integration', exact: true }).click()
    await page.getByRole('button', { name: windows ? 'Enable' : 'Install', exact: true }).click()
    assert.equal(await page.evaluate(name => window.calls.filter(c => c.name === name).length, windows ? 'install_windows_shell_integration' : 'install_finder_location'), 1)
    assert.equal(await page.evaluate(name => window.calls.filter(c => c.name === name).length, windows ? 'install_finder_location' : 'install_windows_shell_integration'), 0)
    await page.screenshot({ path: path.join(output, `${host}-integration.png`), fullPage: true })
    await writeFile(path.join(output, `${host}-calls.json`), JSON.stringify(await page.evaluate(() => window.calls), null, 2))
    cases++; await page.close()
  }
  for (const host of ['windows-nsis', 'macos']) {
    const page = await pageFor(host, 'advanced')
    await page.getByRole('radiogroup', { name: 'Theme', exact: true }).waitFor()
    assert.equal(await page.getByRole('radiogroup', { name: 'Local cache limit', exact: true }).count(), host === 'macos' ? 1 : 0)
    if (host.startsWith('windows')) await assertNotice(page)
    await page.screenshot({ path: path.join(output, `${host}-advanced.png`), fullPage: true })
    cases++; await page.close()
  }
  const onboarding = await pageFor('windows-msi', 'onboarding')
  await onboarding.getByRole('heading', { name: 'Files download when you open them' }).waitFor()
  assert.equal(await onboarding.getByRole('button', { name: 'Smart', exact: true }).count(), 0)
  await onboarding.getByRole('button', { name: /Continue/ }).click()
  assert.equal(await onboarding.evaluate(() => document.body.dataset.continued), 'true')
  assert.equal(await onboarding.evaluate(() => window.calls.filter(c => c.name === 'set_sync_mode').length), 0)
  await onboarding.screenshot({ path: path.join(output, 'windows-onboarding.png'), fullPage: true })
  cases++; await onboarding.close()
  for (const host of ['linux-deb', 'unknown']) {
    const page = await pageFor(host, 'route', { route: 'onboarding' })
    await page.getByRole('button', { name: 'Open web app' }).waitFor()
    assert.equal(await page.getByRole('button', { name: /Native action/ }).count(), 0)
    await assertNotice(page)
    await page.getByRole('button', { name: 'Open web app' }).click()
    assert.equal(await page.evaluate(() => window.calls.filter(c => c.name === 'plugin:opener|open_url' && c.args.url === 'https://app.beebeeb.io').length), 1)
    await page.screenshot({ path: path.join(output, `${host}-alternative.png`), fullPage: true })
    cases++; await page.close()
  }
  const failed = await pageFor('windows-nsis', 'settings', { reject: true })
  await failed.getByRole('button', { name: 'Retry', exact: true }).waitFor()
  assert.equal(await failed.getByRole('button', { name: 'Free up space', exact: true }).count(), 0)
  await assertNotice(failed)
  assert.equal(await failed.getByRole('alert').count(), 1)
  await failed.screenshot({ path: path.join(output, 'capability-error.png'), fullPage: true })
  await failed.getByRole('button', { name: 'Retry', exact: true }).click()
  await failed.waitForFunction(() => window.calls.filter(c => c.name === 'desktop_capabilities').length === 2)
  await failed.getByRole('button', { name: 'Free up space', exact: true }).waitFor()
  assert.equal(await failed.getByRole('alert').count(), 0)
  cases++; await failed.close()
  for (const host of ['linux-deb', 'unknown', 'windows-nsis']) {
    const page = await pageFor(host, 'route', { route: 'onboarding', reject: host === 'windows-nsis', dark: true, viewport: { width: 360, height: 600 } })
    await page.getByRole('button', { name: 'Open web app' }).waitFor()
    await assertNotice(page)
    // Keyboard users can reach every recovery action at compact window widths.
    await page.keyboard.press('Tab')
    assert.equal(await page.locator('button:focus').count(), 1)
    await page.screenshot({ path: path.join(output, `${host}-compact-dark.png`), fullPage: true })
    cases++; await page.close()
  }
  assert.equal(cases, 12)
  console.log(`fixture cases: ${cases} passed; 0 failed; native backend cases: 0`)
} finally { await browser.close() }
