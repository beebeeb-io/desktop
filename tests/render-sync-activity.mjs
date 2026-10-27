// 0.8.15 browser evidence for Home, Settings › Sync, the Windows tray popover and Selective sync, against the Vite dev
// server with a synthetic Tauri IPC (no app, no account, no network). Counts only; the folder names are synthetic.
//
//   bunx vite --port 5291 --strictPort &
//   PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs node tests/render-sync-activity.mjs http://localhost:5291 OUT
//
// What it proves: the surfaces render the model's words for each state, the Settings button calls the menu's own
// pause commands and follows a toggle made elsewhere (an `engine-status` event), a failed resume says the menu's
// sentence, and Selective sync lists one row per folder path. What it does not: WebView2, the native menu, the tray
// icon, or real engine timing (NATIVE rungs).
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [base, output] = process.argv.slice(2)
assert(base && output, 'usage: node tests/render-sync-activity.mjs BASE_URL OUTPUT_DIR')
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))['windows-nsis']
const onHoldFixture = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/sync-activity-summary.on-hold.json'), 'utf8'))
let trees = null
try {
  trees = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/selective-sync-tree.json'), 'utf8'))
} catch {
  trees = null
}
await mkdir(output, { recursive: true })

const HELD = '2,924 changes from 4 Oct are on hold. An update will repair them.'
const GAVE_UP = '43 changes stopped after repeated errors and are not retried.'
const now = Math.floor(Date.now() / 1000)
const summaries = {
  'on-hold': { ...onHoldFixture, engine_state: 'idle', startup_check: null, last_tick_ok_at: now - 120 },
  checking: { ...onHoldFixture },
  syncing: { ...onHoldFixture, engine_state: 'syncing', files_remaining: 12, transfers_active: 3, startup_check: null, last_tick_ok_at: now - 30 },
  'up-to-date': { paused: false, engine_state: 'idle', files_remaining: null, transfers_active: 0, last_tick_ok_at: now - 20, queue: { waiting: 0, gave_up: 0, held: [] }, startup_check: null },
  paused: { ...onHoldFixture, paused: true, engine_state: 'paused', startup_check: null, queue: { ...onHoldFixture.queue, waiting: 7 } },
}

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
const results = []

async function open({ surface, summary, viewport, colorScheme = 'light', failResume = false, tree = null, statusPatch = {}, recent = [] }) {
  const page = await browser.newPage({ viewport, colorScheme })
  const errors = []
  page.on('pageerror', (error) => errors.push(String(error)))
  await page.addInitScript(({ caps, summary, surface, tree, failResume, statusPatch, recent }) => {
    window.calls = []
    window.fixtureSummary = summary
    const callbacks = new Map()
    const listeners = []
    let next = 1
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
    window.emitFixtureEvent = (event, payload) => {
      for (const l of listeners.filter((l) => l.event === event)) callbacks.get(l.handler)?.({ event, id: l.id, payload })
    }
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: surface }, currentWebview: { label: surface } },
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke: async (name, args) => {
        window.calls.push({ name, args })
        const s = window.fixtureSummary
        switch (name) {
          case 'plugin:event|listen': listeners.push({ event: args.event, handler: args.handler, id: next }); return next++
          case 'plugin:event|unlisten': return null
          case 'desktop_capabilities': return caps
          case 'desktop_platform': return 'windows'
          case 'sync_status': return { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0, session_revision: 1, ...statusPatch }
          case 'sync_activity_summary': return s
          case 'tray_pause_sync':
          case 'tray_resume_sync': {
            const pausing = name === 'tray_pause_sync'
            if (!pausing && window.fixtureFailResume) throw new Error('fixture: could not write desktop.toml')
            window.fixtureSummary = { ...s, paused: pausing, engine_state: pausing ? 'paused' : 'idle' }
            window.emitFixtureEvent('engine-status', { state: pausing ? 'paused' : 'idle' })
            return null
          }
          case 'desktop_config': return { theme: 'system', pause_sync: s?.paused ?? false, local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0, metered: false, files_on_demand: true, sync_overlays: true }
          case 'desktop_storage_summary': return { used_bytes: 48_300_000_000, quota_bytes: 200_000_000_000, cache_bytes: 1_200_000_000, pinned_bytes: 2_100_000_000 }
          case 'desktop_file_overview': return { total_files: 0, total_bytes: 0, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
          case 'desktop_recent_files': return recent
          case 'list_vault_folders': return tree ?? []
          case 'get_selective_sync': return []
          case 'get_known_folder_onboarding_seen': return true
          case 'get_known_folder_backup': return []
          case 'account_usage': return { storage_used_bytes: 48_300_000_000, storage_quota_bytes: 200_000_000_000 }
          case 'account_region': return { preferred_region: null, regions: [], choices: [] }
          default:
            if (name.startsWith('plugin:')) return null
            throw new Error(`fixture: no ${name}`)
        }
      },
    }
    window.fixtureFailResume = failResume
  }, { caps, summary, surface, tree, failResume, statusPatch, recent })
  return { page, errors }
}

async function shot(page, name) {
  const file = path.join(output, `${name}.png`)
  await page.screenshot({ path: file, fullPage: false })
  return file
}

async function settingsSync(name, summary, { colorScheme, failResume } = {}) {
  const { page, errors } = await open({ surface: 'main-app', summary, viewport: { width: 1200, height: 860 }, colorScheme, failResume })
  await page.goto(`${base}/?window=main-app&platform=windows`)
  await page.getByRole('button', { name: 'Settings', exact: true }).click()
  await page.getByRole('button', { name: 'Sync', exact: true }).first().click()
  await page.getByRole('heading', { name: 'Sync' }).or(page.getByText('Manage what syncs with this PC', { exact: false })).first().waitFor()
  return { page, errors }
}

function expectText(page, text) {
  return page.getByText(text, { exact: true }).first().waitFor({ timeout: 5000 })
}

async function home(summary, { colorScheme } = {}) {
  const { page, errors } = await open({ surface: 'main-app', summary, viewport: { width: 1200, height: 860 }, colorScheme })
  await page.goto(`${base}/?window=main-app&platform=windows`)
  await page.getByText('Everything on this PC, at a glance.', { exact: false }).waitFor()
  return { page, errors }
}

/** A Home strip figure, once the summary has been read: the value under its label, exactly. */
async function expectHomeFigure(page, label, value) {
  const figure = page.getByText(label, { exact: true }).first().locator('xpath=following-sibling::div[1]')
  const deadline = Date.now() + 5000
  let seen = null
  while (Date.now() < deadline) {
    seen = (await figure.textContent())?.trim() ?? null
    if (seen === value) return
    await page.waitForTimeout(100)
  }
  assert.fail(`Home ${label}: expected "${value}", saw "${seen}"`)
}

try {
  // ── Home (the main window's first page) ────────────────────────────────────
  // On Guus's PC Home said "Up to date · Syncing 0" over 2,924 held and 43 given-up changes. Now: the model's label, and
  // the changes not sent (2,924 + 43).
  for (const [state, figures, colorScheme] of [
    ['on-hold', { State: 'Not in sync', 'Not sent': '2,967' }, 'light'],
    ['on-hold', { State: 'Not in sync', 'Not sent': '2,967' }, 'dark'],
    ['checking', { State: 'Checking', 'Not sent': '2,967' }, 'light'],
    ['up-to-date', { State: 'Up to date', 'Not sent': '0' }, 'light'],
  ]) {
    const name = `home-${state}${colorScheme === 'dark' ? '-dark' : ''}`
    const { page, errors } = await home(summaries[state], { colorScheme })
    for (const [label, value] of Object.entries(figures)) await expectHomeFigure(page, label, value)
    assert.equal(await page.getByText('Syncing', { exact: true }).count(), 0, 'Home has no "Syncing" figure')
    if (state !== 'up-to-date') assert.equal(await page.getByText('Up to date', { exact: true }).count(), 0)
    results.push([`${name} ${JSON.stringify(figures)}`, await shot(page, name)])
    assert.deepEqual(errors, [])
    await page.close()
  }


  // ── Settings › Sync ────────────────────────────────────────────────────────
  {
    const { page, errors } = await settingsSync('settings-sync-on-hold', summaries['on-hold'])
    await expectText(page, HELD)
    await expectText(page, GAVE_UP)
    // Coordinator ruling (26 Oct): the state word is "Not in sync" (chip and State figure), never "On hold".
    assert.equal(await page.getByText('Not in sync', { exact: true }).count(), 2)
    assert.equal(await page.getByText('On hold', { exact: true }).count(), 0)
    assert.equal(await page.getByText('Pause syncing from the app menu', { exact: false }).count(), 0)
    assert.equal(await page.getByText('Your files are synced', { exact: false }).count(), 0)
    const pause = page.getByRole('button', { name: 'Pause syncing', exact: true })
    await pause.waitFor()
    results.push(['settings-sync-on-hold', await shot(page, 'settings-sync-on-hold')])

    // Pause from Settings: the menu's own command, then the surface follows.
    await pause.click()
    await page.getByRole('button', { name: 'Resume syncing', exact: true }).waitFor({ timeout: 5000 })
    await expectText(page, 'Sync is paused.')
    const pauseCalls = await page.evaluate(() => window.calls.filter((c) => c.name === 'tray_pause_sync' || c.name === 'tray_resume_sync').map((c) => c.name))
    assert.deepEqual(pauseCalls, ['tray_pause_sync'])
    results.push(['settings-sync-paused', await shot(page, 'settings-sync-paused')])

    // Resumed from the app menu (another surface): only the event arrives; Settings follows without a click.
    await page.evaluate(() => {
      window.fixtureSummary = { ...window.fixtureSummary, paused: false, engine_state: 'idle' }
      window.emitFixtureEvent('engine-status', { state: 'idle' })
    })
    const t0 = Date.now()
    await page.getByRole('button', { name: 'Pause syncing', exact: true }).waitFor({ timeout: 2000 })
    const followMs = Date.now() - t0
    assert.ok(followMs < 2000, `Settings followed a menu toggle in ${followMs} ms`)
    results.push([`settings-sync-menu-resumed (followed in ${followMs} ms, poll is 3000 ms)`, await shot(page, 'settings-sync-menu-resumed')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  {
    const { page, errors } = await settingsSync('settings-sync-checking', summaries.checking)
    await expectText(page, 'Checking files on this PC: 12,400 of 37,169.')
    await page.getByRole('progressbar', { name: 'Startup check' }).waitFor()
    results.push(['settings-sync-checking', await shot(page, 'settings-sync-checking')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  {
    const { page, errors } = await settingsSync('settings-sync-resume-failed', summaries.paused, { failResume: true })
    await expectText(page, 'Sync is paused.')
    await expectText(page, '7 changes wait until you resume.')
    await page.getByRole('button', { name: 'Resume syncing', exact: true }).click()
    await expectText(page, 'Beebeeb couldn’t resume syncing, so it stays paused.')
    await page.getByRole('button', { name: 'Resume syncing', exact: true }).waitFor()
    results.push(['settings-sync-resume-failed', await shot(page, 'settings-sync-resume-failed')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  {
    const { page, errors } = await settingsSync('settings-sync-up-to-date', summaries['up-to-date'])
    await expectText(page, 'Your files are synced.')
    results.push(['settings-sync-up-to-date', await shot(page, 'settings-sync-up-to-date')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  {
    const { page, errors } = await settingsSync('settings-sync-on-hold-dark', summaries.checking, { colorScheme: 'dark' })
    await expectText(page, HELD)
    results.push(['settings-sync-checking-dark', await shot(page, 'settings-sync-checking-dark')])
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── Tray popover ───────────────────────────────────────────────────────────
  const tray = [
    ['on-hold', HELD],
    ['checking', 'Checking files on this PC: 12,400 of 37,169.'],
    ['syncing', '12 files syncing.'],
    ['paused', 'Sync is paused.'],
    ['up-to-date', 'Your files are synced.'],
  ]
  // With the engine lane's non-blocking guard: uploads run during the pass, read live from `sync_status.startup_guard`.
  {
    const { page, errors } = await open({
      surface: 'tray',
      summary: { ...summaries['on-hold'], transfers_active: 3, startup_check: null },
      viewport: { width: 400, height: 560 },
      statusPatch: { startup_guard: { running: true, done: 2000, total: 37169 } },
    })
    await page.goto(`${base}/?window=tray`)
    await expectText(page, '3 files syncing.')
    await expectText(page, 'Checking files on this PC: 2,000 of 37,169.')
    await page.getByRole('progressbar', { name: 'Startup check' }).waitFor()
    results.push(['tray-uploading-during-check', await shot(page, 'tray-uploading-during-check')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  // Task 2243: the tray's recent list comes from `desktop_recent_files` (the newest rows only), never from the file
  // overview, which loads every row for its totals.
  {
    const recent = [
      { path: 'Documents/Plans/Quarter plan.docx', size_bytes: 24_576, status: 'local', pinned: false, modified_at: now - 60 },
      { path: 'Photos/Holiday/Beach.jpg', size_bytes: 2_400_000, status: 'cloud_only', pinned: true, modified_at: now - 3600 },
      { path: 'Documents/Old notes.txt', size_bytes: 0, status: 'moved_to_trash', pinned: false, modified_at: now - 7200, activity_type: 'moved_to_trash' },
    ]
    const { page, errors } = await open({ surface: 'tray', summary: summaries['up-to-date'], viewport: { width: 400, height: 560 }, recent })
    await page.goto(`${base}/?window=tray`)
    for (const name of ['Quarter plan.docx', 'Beach.jpg', 'Old notes.txt']) await expectText(page, name)
    const calls = await page.evaluate(() => window.calls)
    const reads = calls.filter((c) => c.name === 'desktop_recent_files')
    assert.ok(reads.length >= 1, 'the tray reads desktop_recent_files')
    assert.deepEqual(reads[0].args, { recentLimit: 50 })
    assert.equal(calls.filter((c) => c.name === 'desktop_file_overview').length, 0, 'the tray never reads the file overview')
    results.push([`tray-recent-list (desktop_recent_files x${reads.length}, desktop_file_overview x0)`, await shot(page, 'tray-recent-list')])
    assert.deepEqual(errors, [])
    await page.close()
  }
  for (const [state, headline] of tray) {
    for (const colorScheme of state === 'on-hold' ? ['light', 'dark'] : ['light']) {
      const name = `tray-${state}${colorScheme === 'dark' ? '-dark' : ''}`
      const { page, errors } = await open({ surface: 'tray', summary: summaries[state], viewport: { width: 400, height: 560 }, colorScheme })
      await page.goto(`${base}/?window=tray`)
      await expectText(page, headline)
      if (state !== 'up-to-date') assert.equal(await page.getByText('Your files are synced', { exact: false }).count(), 0)
      if (state === 'on-hold' || state === 'checking' || state === 'syncing' || state === 'paused') await expectText(page, GAVE_UP)
      results.push([name, await shot(page, name)])
      assert.deepEqual(errors, [])
      await page.close()
    }
  }

  // ── Selective sync (one row per path) ──────────────────────────────────────
  if (trees) {
    for (const which of ['before', 'after']) {
      const name = `selective-sync-${which}`
      const { page, errors } = await open({ surface: 'main-app', summary: summaries['up-to-date'], viewport: { width: 1200, height: 860 }, tree: trees[which] })
      await page.goto(`${base}/?window=main-app&platform=windows`)
      await page.getByRole('button', { name: 'Selective sync', exact: true }).first().click()
      await page.getByText('Documents', { exact: true }).first().waitFor({ timeout: 5000 })
      const counts = {}
      for (const folder of ['Documents', 'Photos', 'Backups', 'Music']) counts[folder] = await page.getByText(folder, { exact: true }).count()
      await writeFile(path.join(output, `${name}-counts.json`), JSON.stringify(counts, null, 2))
      if (which === 'after') for (const [folder, n] of Object.entries(counts)) assert.equal(n, 1, `${folder} is listed ${n} times`)
      else assert.ok(Object.values(counts).some((n) => n > 1), 'the before tree lists a folder twice')
      results.push([`${name} ${JSON.stringify(counts)}`, await shot(page, name)])
      assert.deepEqual(errors, [])
      await page.close()
    }
  }
} finally {
  await browser.close()
}
for (const [what, file] of results) console.log(`ok  ${what}  ${file}`)
console.log(`browser cases: ${results.length} passed; 0 failed`)
