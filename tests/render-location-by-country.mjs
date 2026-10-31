// Task 2050 (storage location by COUNTRY): the real Data residency UI of the macOS Settings window and the
// Windows/Linux Settings panel in headless Chromium, against a MOCKED server (no network, no API).
//
//   PLAYWRIGHT_MODULE=$HOME/.bun/install/global/node_modules/playwright/index.mjs \
//     node tests/render-location-by-country.mjs REPO OUTPUT
//
// The synthetic part is the IPC bridge: `account_region` / `account_set_region` are answered by an in-memory model
// of the server contract (server PR #251, aead1d01): the list `europe` / `de` / `fi` / `nl` (nl coming soon), a
// PUT that refuses a coming-soon or unknown id with 400 `region_unavailable`, `region_strict` kept only for a
// country. The bridge applies the same mapping the Rust DTO applies (`available_regions` -> `regions`,
// `example_country` -> `country`, NO city read). The mocked server still sends the deprecated `city` /
// `example_city` keys, to prove the UI never shows them. An OLDER server (one `datacenter` row per datacenter,
// each with a city) is run as a second flavour. What this is NOT: the native shell, the Rust commands, a Mac, or
// the real server (which is not deployed). 1440 x 900 for the Windows/Linux panel; the macOS window at its own 560 wide.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-location-by-country.mjs REPO OUTPUT')
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

// ── the mocked server ───────────────────────────────────────────────────────
const country = (id, name, code, status = 'live') => ({ id, kind: 'country', name, country: name, country_code: code, status, selectable: status === 'live', is_default: false })
const europe = { id: 'europe', kind: 'region', name: 'Europe', country: null, country_code: null, status: 'live', selectable: true, is_default: true }
const FLAVOURS = {
  country: [europe, country('de', 'Germany', 'DE'), country('fi', 'Finland', 'FI'), country('nl', 'Netherlands', 'NL', 'coming_soon')],
  // An older server: one row per datacenter, named after its city, each with a city key.
  older: [
    europe,
    { id: 'fsn1', kind: 'datacenter', name: 'Falkenstein', city: 'Falkenstein', country: 'Germany', country_code: 'DE', status: 'live', selectable: true, is_default: false },
    { id: 'hel1', kind: 'datacenter', name: 'Helsinki', city: 'Helsinki', country: 'Finland', country_code: 'FI', status: 'live', selectable: true, is_default: false },
    { id: 'nbg1', kind: 'datacenter', name: 'Nuremberg', city: 'Nuremberg', country: 'Germany', country_code: 'DE', status: 'live', selectable: true, is_default: false },
    { id: 'ede', kind: 'datacenter', name: 'Ede', city: 'Ede', country: 'Netherlands', country_code: 'NL', status: 'coming_soon', selectable: false, is_default: false },
  ],
}
function makeServer(flavour, stored = { preferred_region: null, region_strict: false }) {
  const state = { ...stored }
  const choices = FLAVOURS[flavour]
  return {
    state,
    api(method, route, body) {
      if (route !== '/api/v1/me/region') return { status: 404, text: '{}' }
      if (method === 'GET') {
        return { status: 200, text: JSON.stringify({ ...state, available_regions: [{ continent: 'europe', display_name: 'Europe', example_city: 'Falkenstein', example_country: 'Germany', is_default: true }], choices }) }
      }
      const want = body.preferred_region
      const hit = want === null ? null : choices.find((c) => c.id === String(want).toLowerCase())
      if (want !== null && (!hit || !hit.selectable)) return { status: 400, text: JSON.stringify({ error: 'region_unavailable', message: `'${want}' is not available` }) }
      const strict = body.region_strict === true
      if (strict && !(hit && hit.kind !== 'region')) return { status: 400, text: JSON.stringify({ error: 'region_strict_requires_datacenter', message: 'Only a specific country can be strict.' }) }
      state.preferred_region = want
      state.region_strict = strict
      return { status: 200, text: JSON.stringify({ preferred_region: want, region_strict: strict }) }
    },
  }
}

// ── the page side: the IPC bridge ───────────────────────────────────────────
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
          const r = await window.__api('PUT', '/api/v1/me/region', { preferred_region: args.preferredRegion, ...(args.regionStrict === undefined ? {} : { region_strict: args.regionStrict }) })
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
  snapshot: { phase: 'synced', sync_phase: 'synced', generated_at: 1000, paused: false, account: { email: 'region-test@beebeeb.io', logged_in: true, vault_unlocked: true, auth_expired: false },
    engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 990 }, reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
    storage: { used_bytes: 10, quota_bytes: 100, fetched_at: 900, stale: false }, storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [] },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T00:00:00Z', quota_bytes: 100, used_bytes: 10 },
}

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let checks = 0
const ok = (cond, label) => { assert(cond, label); checks += 1 }
const CITY = /Falkenstein|Helsinki|Nuremberg|N[uü]rnberg|\bEde\b/

const SURFACES = {
  mac: {
    host: caps.macos,
    viewport: { width: 560, height: 900 },
    async open(page) {
      await page.route('http://fixture.local/**', (r) => r.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="http://fixture.local/bundle/main.css"></head><body><div id="root"></div><script src="http://fixture.local/bundle/main.js"></script></body></html>` }))
      await page.route('http://fixture.local/bundle/**', async (r) => r.fulfill({ path: path.join(macDir, path.basename(new URL(r.request().url()).pathname)), contentType: r.request().url().endsWith('.css') ? 'text/css' : 'text/javascript' }))
      await page.goto('http://fixture.local/?window=main&platform=macos&tab=account')
      await page.getByRole('radiogroup').waitFor()
    },
    row: (page, name) => page.getByRole('radio', { name: new RegExp(`^${name}\\b`) }),
    isChecked: async (loc) => (await loc.getAttribute('aria-checked')) === 'true',
    rows: (page) => page.getByRole('radio').evaluateAll((els) => els.map((e) => e.getAttribute('data-row'))),
  },
  windows: {
    host: caps['windows-nsis'],
    viewport: { width: 1440, height: 900 },
    async open(page) {
      await page.route('http://fixture.local/**', (r) => r.fulfill({ contentType: 'text/html', body: '<div id="root"></div>' }))
      await page.goto('http://fixture.local/?surface=settings')
      await page.addStyleTag({ content: css })
      await page.addScriptTag({ path: path.join(winDir, 'residency-render.js') })
      await page.getByRole('button', { name: 'Data residency', exact: true }).click()
      await page.getByRole('button', { name: /^Europe\b/ }).waitFor()
    },
    row: (page, name) => page.getByRole('button', { name: new RegExp(`^${name}\\b`) }),
    isChecked: async (loc) => (await loc.getAttribute('aria-pressed')) === 'true',
    rows: (page) => page.locator('button[aria-pressed]').evaluateAll((els) => els.map((e) => e.innerText.split('\n')[0])),
  },
}

try {
  for (const [name, surface] of Object.entries(SURFACES)) {
    const fresh = async (server) => {
      const page = await browser.newPage({ viewport: surface.viewport })
      page.setDefaultTimeout(8000)
      const errors = []
      page.on('pageerror', (e) => errors.push(String(e)))
      await page.exposeFunction('__api', (method, route, body) => server.api(method, route, body))
      await page.addInitScript(installBackend, { host: surface.host, win: winFx })
      await surface.open(page)
      return { page, errors }
    }
    const shot = (page, label) => page.screenshot({ path: path.join(output, 'png', `${name}-${label}.png`) })

    // 1. the country server, no choice: Europe default, Germany, Finland, the Netherlands coming soon
    let server = makeServer('country')
    let { page, errors } = await fresh(server)
    const text = () => page.locator('body').innerText()
    const rows = await surface.rows(page)
    ok(rows.length === 4, `${name}: 4 rows, got ${JSON.stringify(rows)}`)
    ok(await surface.isChecked(surface.row(page, 'Europe')), `${name}: Europe is the checked default`)
    let body = await text()
    for (const label of ['Europe', 'Germany', 'Finland', 'Netherlands', 'Coming soon', 'Default']) ok(body.includes(label), `${name}: shows ${label}`)
    ok(/Stored in the EU\. New files go to any of our European data centers\./.test(body), `${name}: Europe summary`)
    ok(await surface.row(page, 'Netherlands').isDisabled(), `${name}: the Netherlands is disabled`)
    ok(!(await surface.row(page, 'Finland').isDisabled()), `${name}: Finland is selectable`)
    ok(!CITY.test(body), `${name}: no city on screen (the server sent one: ${/Falkenstein/.test(JSON.stringify(server.api('GET', '/api/v1/me/region')))})`)
    ok((await page.getByRole('switch').count()) === 0, `${name}: no strict switch while Europe is chosen`)
    await shot(page, '1-europe-default')

    // 2. choose Finland: the id is sent verbatim, strict stated; the strict switch appears, named after the country
    await surface.row(page, 'Finland').click()
    await page.waitForTimeout(400)
    ok(server.state.preferred_region === 'fi', `${name}: the server stored fi`)
    ok(await surface.isChecked(surface.row(page, 'Finland')), `${name}: Finland is the checked row`)
    body = await text()
    ok(/Stored in the EU\. New files go to Finland\./.test(body), `${name}: Finland summary`)
    ok(body.includes('Only store in Finland'), `${name}: "Only store in Finland" is offered`)
    ok(JSON.stringify(await page.evaluate(() => window.__calls.filter((c) => c.name === 'account_set_region').map((c) => c.args))) === JSON.stringify([{ preferredRegion: 'fi', regionStrict: false }]), `${name}: sent fi with regionStrict false`)
    await shot(page, '2-finland')

    // 3. the Netherlands cannot be chosen
    await surface.row(page, 'Netherlands').click({ force: true }).catch(() => {})
    await page.waitForTimeout(300)
    ok((await page.evaluate(() => window.__calls.filter((c) => c.name === 'account_set_region').length)) === 1, `${name}: clicking the Netherlands sent nothing`)

    // 4. only store in Finland on: the server holds region_strict
    await page.getByRole('switch', { name: 'Only store in Finland' }).click()
    await page.waitForTimeout(400)
    ok(server.state.region_strict === true && server.state.preferred_region === 'fi', `${name}: the server stored fi + strict`)
    ok((await page.getByRole('switch', { name: 'Only store in Finland' }).getAttribute('aria-checked')) === 'true', `${name}: the switch reads on`)
    body = await text()
    ok(body.includes('the upload stops instead of going to another EU country.'), `${name}: the on hint`)
    await shot(page, '3-finland-strict-on')

    // 5. another country keeps strict; Europe ends it
    await surface.row(page, 'Germany').click()
    await page.waitForTimeout(400)
    ok(server.state.preferred_region === 'de' && server.state.region_strict === true, `${name}: strict carried to Germany`)
    ok((await text()).includes('Only store in Germany'), `${name}: the switch is now about Germany`)
    await shot(page, '4-germany-strict-on')
    await surface.row(page, 'Europe').click()
    await page.waitForTimeout(400)
    ok(server.state.preferred_region === 'europe' && server.state.region_strict === false, `${name}: Europe ended strict`)
    ok((await page.getByRole('switch').count()) === 0, `${name}: no strict switch on Europe again`)
    ok(errors.length === 0, `${name}: no page errors ${JSON.stringify(errors)}`)
    await page.close()

    // 6. a stored country + strict is read back after a reload
    server = makeServer('country', { preferred_region: 'de', region_strict: true })
    ;({ page, errors } = await fresh(server))
    ok(await surface.isChecked(surface.row(page, 'Germany')), `${name}: reload shows the stored de`)
    ok((await page.getByRole('switch', { name: 'Only store in Germany' }).getAttribute('aria-checked')) === 'true', `${name}: reload shows strict on`)
    await page.close()

    // 7. an OLDER server (a row per datacenter, each with a city): folded into countries, no city on screen,
    //    a stored datacenter id selects its country
    server = makeServer('older', { preferred_region: 'nbg1', region_strict: false })
    ;({ page, errors } = await fresh(server))
    body = await text()
    const olderRows = await surface.rows(page)
    ok(olderRows.length === 4, `${name}: older server folds to 4 rows, got ${JSON.stringify(olderRows)}`)
    ok(!CITY.test(body), `${name}: older server: no city on screen`)
    ok(await surface.isChecked(surface.row(page, 'Germany')), `${name}: older server: stored nbg1 selects Germany`)
    ok(/Stored in the EU\. New files go to Germany\./.test(body), `${name}: older server: Germany summary`)
    await shot(page, '5-older-server-folded')
    await surface.row(page, 'Finland').click()
    await page.waitForTimeout(400)
    ok(server.state.preferred_region === 'hel1', `${name}: older server: Finland is sent by its datacenter id (${server.state.preferred_region})`)
    ok(errors.length === 0, `${name}: older server: no page errors ${JSON.stringify(errors)}`)
    await page.close()
  }
  console.log(`mocked-IPC UI checks: ${checks} passed; 0 failed; screenshots in ${path.join(output, 'png')}; native shell: not run`)
} finally {
  await browser.close()
}
