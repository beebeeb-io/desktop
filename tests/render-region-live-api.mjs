// Task 1919 (by COUNTRY since task 2050): the real Data residency UI of BOTH settings windows, against a REAL local API
// built from server PR #251 or later (the country list). NOT RUN as part of task 2050: no such API was available.
//
//   BB_REAL_API_BASE=http://127.0.0.1:39191 BB_REAL_API_TOKEN=<session token> \
//   BB_REAL_PSQL='docker exec -i beebeebio-postgres-1 psql -U beebeeb -d bb_1919_desktop -At' \
//   PLAYWRIGHT_MODULE=$HOME/.bun/install/global/node_modules/playwright/index.mjs \
//     node tests/render-region-live-api.mjs REPO OUTPUT
//
// What it is: the shipping React windows (src/main.tsx for macOS Settings, the Windows/Linux SettingsView) in
// headless Chromium. The only synthetic part is the IPC bridge, which maps the two region commands onto the
// real API with the same mapping the Rust DTO applies (`available_regions` -> `regions`, `example_country` ->
// `country`, no city is read; `choices` and `region_strict` as sent) and the same error text shape. What it is NOT: the native shell, the Rust
// commands (those run in `cargo test real_api_region`), or a Mac. It refuses any API that is not loopback.
// Each case proves a UI action by reading the API back, not the UI, and a data change by editing the API's
// scratch database (BB_REAL_PSQL) and reloading with no client change.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-region-live-api.mjs REPO OUTPUT')
const base = process.env.BB_REAL_API_BASE ?? ''
const token = process.env.BB_REAL_API_TOKEN ?? ''
assert(/^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(base), `loopback API only, got "${base}"`)
assert(token, 'BB_REAL_API_TOKEN')
const psql = process.env.BB_REAL_PSQL
assert(psql, 'BB_REAL_PSQL (the API scratch database), to change the API data between loads')
const sql = (statement) => {
  const [cmd, ...args] = psql.split(' ')
  const r = spawnSync(cmd, [...args, '-c', statement], { encoding: 'utf8' })
  assert.equal(r.status, 0, `psql failed: ${r.stderr}`)
  return r.stdout.trim()
}
await mkdir(path.join(output, 'png'), { recursive: true })

const bundle = (entry, name) => {
  const outdir = path.join(output, name)
  const r = spawnSync('bun', ['build', path.join(repo, entry), '--outdir', outdir, '--target', 'browser', '--format', 'iife', '--define', 'process.env.NODE_ENV="development"'], { encoding: 'utf8', cwd: repo })
  assert.equal(r.status, 0, `bun build ${entry} failed: ${r.stderr}`)
  return outdir
}
const macDir = bundle('src/main.tsx', 'bundle-mac')
const winDir = bundle('tests/fixtures/residency-render.tsx', 'bundle-win')
const css = await readFile(path.join(repo, 'src/design.css'), 'utf8')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))

const api = async (method, route, body) => {
  const r = await fetch(`${base}${route}`, { method, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) })
  return { status: r.status, text: await r.text() }
}
const serverPreference = async () => JSON.parse((await api('GET', '/api/v1/me/region')).text).preferred_region

// The page side: the IPC bridge. Region commands go to the real API through `__api`.
function installBackend({ host, win }) {
  window.__calls = []
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback: () => 1,
    unregisterCallback() {},
    invoke: async (name, args) => {
      window.__calls.push({ name, args })
      switch (name) {
        case 'account_region': {
          const r = await window.__api('GET', '/api/v1/me/region')
          if (r.status !== 200) throw new Error(`HTTP ${r.status}: ${r.text}`)
          const j = JSON.parse(r.text)
          return {
            preferred_region: j.preferred_region ?? null,
            region_strict: j.region_strict === true,
            regions: j.available_regions.map((x) => ({ continent: x.continent, display_name: x.display_name, country: x.example_country ?? null, is_default: x.is_default })),
            choices: (j.choices ?? []).map(({ city, ...rest }) => rest),
          }
        }
        case 'account_set_region': {
          const r = await window.__api('PUT', '/api/v1/me/region', { preferred_region: args.preferredRegion })
          if (r.status !== 200) throw new Error(`HTTP ${r.status}: ${r.text}`)
          return JSON.parse(r.text)
        }
        case 'desktop_capabilities': return host
        case 'desktop_platform': return host.host_os
        case 'app_version': return '0.8.6'
        case 'desktop_config': case 'get_desktop_config': return { theme: 'system', upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, local_cache_limit_bytes: 0 }
        case 'sync_status': return { session_revision: 1, logged_in: true, vault_unlocked: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: false }
        case 'popover_snapshot': return win.snapshot
        case 'account_subscription': return win.subscription
        case 'autostart_enabled': return true
        case 'finder_setup_state': return { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }
        case 'list_remote_tree': return []
        default:
          if (name.startsWith('plugin:')) return 1
          return null
      }
    },
  }
}
const winFx = {
  snapshot: { phase: 'synced', generated_at: 1000, paused: false, account: { email: 'region-test@beebeeb.io', logged_in: true, vault_unlocked: true, auth_expired: false },
    engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 990 }, reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
    storage: { used_bytes: 10, quota_bytes: 100, fetched_at: 900, stale: false }, storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [] },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T00:00:00Z', quota_bytes: 100, used_bytes: 10 },
}

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let checks = 0
const ok = (cond, label) => { assert(cond, label); checks += 1 }

// A surface = how to open it and how to read/click its rows.
const SURFACES = {
  mac: {
    host: caps.macos,
    async open(page) {
      await page.route('http://fixture.local/**', (r) => r.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="http://fixture.local/bundle/main.css"></head><body><div id="root"></div><script src="http://fixture.local/bundle/main.js"></script></body></html>` }))
      await page.route('http://fixture.local/bundle/**', async (r) => r.fulfill({ path: path.join(macDir, path.basename(new URL(r.request().url()).pathname)), contentType: r.request().url().endsWith('.css') ? 'text/css' : 'text/javascript' }))
      await page.goto('http://fixture.local/?window=settings-v2&platform=macos&tab=account')
      await page.getByRole('radiogroup').waitFor()
    },
    row: (page, name) => page.getByRole('radio', { name: new RegExp(`^${name}\\b`) }),
    checked: async (page) => page.getByRole('radio', { checked: true }).evaluateAll((els) => els.map((e) => e.getAttribute('data-row'))),
    isChecked: async (loc) => (await loc.getAttribute('aria-checked')) === 'true',
    rows: (page) => page.getByRole('radio').evaluateAll((els) => els.map((e) => e.getAttribute('data-row'))),
  },
  windows: {
    host: caps['windows-nsis'],
    async open(page) {
      await page.route('http://fixture.local/**', (r) => r.fulfill({ contentType: 'text/html', body: '<div id="root"></div>' }))
      await page.goto('http://fixture.local/?surface=settings')
      await page.addStyleTag({ content: css })
      await page.addScriptTag({ path: path.join(winDir, 'residency-render.js') })
      await page.getByRole('button', { name: 'Data residency', exact: true }).click()
      await page.getByText('Existing files stay where they are.').waitFor()
    },
    row: (page, name) => page.getByRole('button', { name: new RegExp(`^${name}\\b`) }),
    checked: async (page) => page.locator('button[aria-pressed="true"]').evaluateAll((els) => els.map((e) => e.innerText.split('\n')[0])),
    isChecked: async (loc) => (await loc.getAttribute('aria-pressed')) === 'true',
    rows: (page) => page.locator('button[aria-pressed]').evaluateAll((els) => els.map((e) => e.innerText.split('\n')[0])),
  },
}

// The Linux build shows the same SettingsView as Windows; only its capability set differs.
SURFACES.linux = { ...SURFACES.windows, host: caps['linux-appimage'] }

try {
  for (const [name, surface] of Object.entries(SURFACES)) {
    const fresh = async () => {
      const page = await browser.newPage({ viewport: name === 'mac' ? { width: 560, height: 900 } : { width: 1200, height: 900 } })
      page.setDefaultTimeout(8000)
      const errors = []
      page.on('pageerror', (e) => errors.push(String(e)))
      await page.exposeFunction('__api', (method, route, body) => api(method, route, body))
      await page.addInitScript(installBackend, { host: surface.host, win: winFx })
      await surface.open(page)
      return { page, errors }
    }

    // 0. start from "no choice": the API's default
    assert.equal((await api('PUT', '/api/v1/me/region', { preferred_region: null })).status, 200)
    let { page, errors } = await fresh()
    const text = () => page.locator('body').innerText()

    // 1. the list is the API's: four rows, in the API's order, Europe checked, the Netherlands coming soon and disabled
    const rows = await surface.rows(page)
    ok(rows.length === 4, `${name}: 4 rows, got ${JSON.stringify(rows)}`)
    ok((await surface.checked(page)).length === 1, `${name}: exactly one row checked`)
    ok(await surface.isChecked(surface.row(page, 'Europe')), `${name}: Europe is the checked default`)
    const body1 = await text()
    ok(/Coming soon/.test(body1), `${name}: the Netherlands says Coming soon`)
    ok(/Stored in the EU\. New files go to any of our European data centers\./.test(body1), `${name}: Europe summary`)
    ok(await surface.row(page, 'Netherlands').isDisabled(), `${name}: the Netherlands is disabled`)
    ok(!(await surface.row(page, 'Finland').isDisabled()), `${name}: Finland is selectable`)
    ok(!/hetzner|provider|endpoint/i.test(body1), `${name}: no provider words`)
    ok(!/Falkenstein|Helsinki|Nuremberg|\bEde\b/.test(body1), `${name}: no city`)
    await page.screenshot({ path: path.join(output, 'png', `${name}-1-europe-default.png`) })

    // 2. choose Finland: the API now holds fi (read from the API, not the UI)
    await surface.row(page, 'Finland').click()
    await page.waitForFunction(() => window.__calls.some((c) => c.name === 'account_set_region'))
    await page.waitForTimeout(400)
    ok((await serverPreference()) === 'fi', `${name}: the API stored fi`)
    ok(await surface.isChecked(surface.row(page, 'Finland')), `${name}: Finland is the checked row`)
    ok(/Stored in the EU\. New files go to Finland\./.test(await text()), `${name}: Finland summary from the payload`)
    const sent = await page.evaluate(() => window.__calls.filter((c) => c.name === 'account_set_region').map((c) => c.args))
    ok(JSON.stringify(sent) === JSON.stringify([{ preferredRegion: 'fi', regionStrict: false }]), `${name}: sent the id verbatim ${JSON.stringify(sent)}`)
    await page.screenshot({ path: path.join(output, 'png', `${name}-2-finland.png`) })

    // 3. the Netherlands cannot be chosen: nothing is sent, the API keeps fi
    await surface.row(page, 'Netherlands').click({ force: true }).catch(() => {})
    await page.waitForTimeout(300)
    const sentAfterNl = await page.evaluate(() => window.__calls.filter((c) => c.name === 'account_set_region').length)
    ok(sentAfterNl === 1, `${name}: clicking the Netherlands sent nothing (${sentAfterNl} set calls)`)
    ok((await serverPreference()) === 'fi', `${name}: still fi after the Netherlands`)

    // 3b. only store in Finland: the API holds region_strict
    await page.getByRole('switch', { name: 'Only store in Finland' }).click()
    await page.waitForTimeout(400)
    ok(JSON.parse((await api('GET', '/api/v1/me/region')).text).region_strict === true, `${name}: the API stored region_strict`)
    await page.screenshot({ path: path.join(output, 'png', `${name}-2b-finland-strict.png`) })

    // 4. back to Europe: the API holds "europe", and strict ends with it
    await surface.row(page, 'Europe').click()
    await page.waitForTimeout(500)
    ok((await serverPreference()) === 'europe', `${name}: the API stored europe`)
    ok(JSON.parse((await api('GET', '/api/v1/me/region')).text).region_strict === false, `${name}: strict ended with Europe`)
    ok(await surface.isChecked(surface.row(page, 'Europe')), `${name}: Europe is checked again`)
    ok(errors.length === 0, `${name}: no page errors ${JSON.stringify(errors)}`)
    await page.close()

    // 5. a stored country shows as chosen after a reload (read from the API), then the API data changes
    assert.equal((await api('PUT', '/api/v1/me/region', { preferred_region: 'de' })).status, 200)
    ;({ page, errors } = await fresh())
    ok(await surface.isChecked(surface.row(page, 'Germany')), `${name}: reload shows the stored de`)
    await page.close()

    // 6. NO CLIENT CHANGE: take Finland's datacenter out of service in the API's database. The list is the
    //    API's: the reloaded window shows Finland iff the API still lists it as selectable.
    try {
      sql("UPDATE storage_pools SET is_healthy=FALSE WHERE region='hel1'")
      const list = JSON.parse((await api('GET', '/api/v1/storage-regions')).text)
      const changed = JSON.stringify(list.map((x) => [x.id, x.name, x.selectable]))
      ;({ page, errors } = await fresh())
      const after = await surface.rows(page)
      await page.screenshot({ path: path.join(output, 'png', `${name}-3-api-data-changed.png`) })
      const finland = list.find((x) => x.id === 'fi')
      ok((await text()).includes('Finland') === Boolean(finland), `${name}: Finland shown iff the API lists it (${changed})`)
      if (finland) ok((await surface.row(page, 'Finland').isDisabled()) === !finland.selectable, `${name}: Finland disabled iff the API says not selectable (${changed})`)
      console.log(`${name}: after the API data change the API lists ${changed}; the window shows ${JSON.stringify(after)}`)
      await page.close()
    } finally {
      sql("UPDATE storage_pools SET is_healthy=TRUE WHERE region='hel1'")
    }
    await api('PUT', '/api/v1/me/region', { preferred_region: null })
  }
  console.log(`live-API UI checks: ${checks} passed; 0 failed; screenshots in ${path.join(output, 'png')}; native shell: not run`)
} finally {
  await browser.close()
}
