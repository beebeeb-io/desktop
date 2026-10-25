// Real-browser check of the macOS Beebeeb window (task 1683 slice 1; spec docs/specs/2026-10-24-macos-two-surface-app.md
// §7, §8, §11; hi-fi design/hifi/macos-two-surface.html sections 3 to 5). Local evidence, not a CI gate (Playwright is
// not a desktop dependency); tests/macApp.test.tsx and tests/macSettingsLayout.test.ts pin the same rules in CI.
//
//   node tests/render-mac-app.mjs REPO OUTPUT
//   PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs   (as the other render-*.mjs)
//   HIFI_PNG_DIR=/path/to/hifi-figures               optional: side by side with the hi-fi's figures
//
// It bundles the REAL src/main.tsx (the mount path the app uses), loads `?window=main&platform=macos` with a synthetic
// Tauri IPC, drives the window to each state through its own controls (the gear, the sidebar, the buttons), and
// checks: nothing that clips or scrolls is wider than its box; the document never scrolls; the window shows the words
// it must and none it must not; no page errors. Each state is drawn at 920 x 620 (the default size) in light and dark,
// and the Settings states again at 760 x 520 (the minimum). A drawn copy of the traffic lights sits over the sidebar in
// the PNGs only. Exits 1 on any failed check, else prints a count line.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-mac-app.mjs REPO OUTPUT')
await mkdir(path.join(output, 'png'), { recursive: true })

const outdir = path.join(output, 'bundle')
const built = spawnSync('bun', ['build', path.join(repo, 'src/main.tsx'), '--outdir', outdir, '--target', 'browser', '--format', 'iife', '--define', 'process.env.NODE_ENV="development"'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8')).macos
const pagePath = path.join(output, 'page.html')
await writeFile(pagePath, '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><link rel="stylesheet" href="bundle/main.css"></head><body><div id="root"></div><script src="bundle/main.js"></script></body></html>')

const LONG_EMAIL = `${'x'.repeat(48)}@example.eu` // 60 characters, no break before the @
const KEPT_OURS = '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12'
const KEPT_MACOS = '/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 14.05)'
const snapshot = (over = {}) => ({
  phase: 'synced', generated_at: 1000, paused: false,
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: Math.floor(Date.now() / 1000) - 70 },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false },
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
  ...over,
  account: { email: 'sam@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false, ...(over.account ?? {}) },
})
const base = {
  snapshot: snapshot(),
  finder: { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null },
  config: { upload_kbps_limit: 0, download_kbps_limit: 5000, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'system' },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-11-14T00:00:00Z', quota_bytes: 200_000_000_000, used_bytes: 84_300_000_000 },
  keptUnsent: null, kept: null,
  // The shape `account_region` answers with (Rust's DTO, as tests/render-location-by-country.mjs maps it).
  region: {
    preferred_region: null,
    region_strict: false,
    regions: [{ continent: 'europe', display_name: 'Europe', country: 'Germany', is_default: true }],
    choices: [
      { id: 'europe', kind: 'region', name: 'Europe', country: null, country_code: null, status: 'live', selectable: true, is_default: true },
      { id: 'de', kind: 'country', name: 'Germany', country: 'Germany', country_code: 'DE', status: 'live', selectable: true, is_default: false },
    ],
  },
}

function installBackend({ caps, fx }) {
  window.__fx = fx
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback: () => 1,
    unregisterCallback() {},
    invoke: async (name, a) => {
      const f = window.__fx
      switch (name) {
        case 'desktop_capabilities': return caps
        case 'desktop_platform': return 'macos'
        case 'app_version': return '0.8.12'
        case 'sync_status': return { session_revision: 1, logged_in: f.snapshot.account.logged_in, vault_unlocked: f.snapshot.account.vault_unlocked, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: f.snapshot.account.auth_expired, engine_refusal: null }
        case 'popover_snapshot': return f.snapshot
        case 'account_subscription': return f.subscription
        case 'get_desktop_config': return f.config
        case 'desktop_config': return f.config
        case 'set_desktop_config': f.config = { ...f.config, ...a.config }; return null
        case 'autostart_enabled': return true
        case 'finder_setup_state': return f.finder
        case 'kept_unsent_folder': return f.keptUnsent
        case 'kept_unsynced_folder': return f.kept
        case 'list_remote_tree': return [{ id: 'f1', name: 'Photos', is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: true, children: [] }]
        case 'account_region': return f.region
        case 'consume_main_navigation': return null
        case 'consume_menu_update_check': return false
        case 'consume_sign_out_flow_request': return false
        case 'main_window_state': return null
        case 'sign_out_count': return { askId: 'a0', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }
        default:
          if (name.startsWith('plugin:')) return 1
          return null
      }
    },
  }
}

function report() {
  const scrolling = document.scrollingElement
  const clips = (el) => {
    const s = getComputedStyle(el)
    if (s.textOverflow === 'ellipsis') return false
    return [s.overflowX, s.overflowY].some((v) => v !== 'visible')
  }
  const name = (el) => (el === document.documentElement ? 'html' : `${el.tagName.toLowerCase()}${typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).join('.') : ''}`)
  const containers = [document.documentElement, document.body, ...document.querySelectorAll('body *')].filter((el) => el === document.documentElement || el === document.body || clips(el))
  const rows = containers.filter((el) => el.clientWidth > 0).map((el) => ({
    el: name(el),
    scrollWidth: el === document.documentElement ? scrolling.scrollWidth : el.scrollWidth,
    clientWidth: el === document.documentElement ? scrolling.clientWidth : el.clientWidth,
  }))
  const sidebar = document.querySelector('.mw-sidebar')
  return {
    innerHeight, docScrollHeight: scrolling.scrollHeight,
    wide: rows.filter((r) => r.scrollWidth > r.clientWidth),
    rows: rows.length,
    sidebarWidth: sidebar ? Math.round(sidebar.getBoundingClientRect().width) : null,
    barHeight: (() => { const b = document.querySelector('.mw-storage .ms-bar'); return b ? Math.round(b.getBoundingClientRect().height) : null })(),
    toolbarHeight: (() => { const t = document.querySelector('.mw-toolbar'); return t ? Math.round(t.getBoundingClientRect().height) : null })(),
    dialogs: document.querySelectorAll('[role="dialog"]').length,
    sidebarButtons: sidebar ? [...sidebar.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.innerText.trim()) : [],
    text: document.body.innerText,
  }
}

const gear = async (p) => { await p.getByRole('button', { name: 'Settings', exact: true }).click() }
const section = (label) => async (p) => { await gear(p); await p.getByRole('button', { name: label }).first().click() }

const SCENARIOS = [
  { name: 'home', bar: true, has: ['Home', 'End-to-end encrypted', 'Up to date', 'Last checked 1 min ago', 'Beebeeb in Finder', 'Your vault appears under Locations in Finder.', 'Open in Finder', 'Storage', '84,3 GB of 200 GB', 'sam@example.eu', 'Basic plan'], hasNot: ['Needs your attention', 'control center', 'daemon'], hifi: 'hifi-s3-fig0' },
  { name: 'home-kept', fx: { keptUnsent: KEPT_OURS, kept: KEPT_MACOS }, has: ['Needs your attention', KEPT_OURS, KEPT_MACOS, 'Show in Finder', 'Dismiss'] },
  { name: 'home-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, has: ['Your vault is locked', 'Unlock vault', 'Vault locked'], hasNot: ['Beebeeb in Finder'], hifi: 'hifi-s3-fig3' },
  { name: 'home-long-email', fx: { snapshot: snapshot({ account: { email: LONG_EMAIL } }) }, has: [LONG_EMAIL] },
  { name: 'settings-general', act: gear, has: ['Settings', 'Home', 'General', 'Open Beebeeb at login', 'Tell me about'], sidebar: ['Back to Home', 'Account, sam@example.eu, Basic plan', 'General', 'Sync', 'About'], min: true, hifi: 'hifi-s4-fig1' },
  { name: 'settings-account', act: section('Account, sam@example.eu, Basic plan'), has: ['Account', 'sam@example.eu', 'Basic plan · renews 14 Nov 2026', 'Vault is unlocked', 'Where your files are stored', 'Europe', 'Germany', 'Sign out of this Mac', 'Sign out…'], hasNot: ['Couldn’t load storage locations'], min: true, hifi: 'hifi-s4-fig2' },
  { name: 'settings-sync', act: section('Sync'), has: ['Beebeeb in Finder', 'Added', 'Repair…', 'Keep on this Mac', 'Speed'], min: true, hifi: 'hifi-s4-fig3' },
  { name: 'settings-about', act: section('About'), has: ['Beebeeb for Mac', 'Version 0.8.12', 'Check for updates', 'Get help', 'Export a support bundle'], min: true, hifi: 'hifi-s4-fig5' },
  { name: 'signout-confirm', act: async (p) => { await section('Account, sam@example.eu, Basic plan')(p); await p.getByRole('button', { name: 'Sign out…' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Sign out of this Mac?', 'Sync stops until you sign in again.', 'Cancel', 'Sign out'], dialogs: 1, hifi: 'hifi-s5-fig0' },
  { name: 'signed-out', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, has: ['Sign in to Beebeeb', 'Sign in'], hasNot: ['Home', 'Settings'], noSidebar: true },
  { name: 'signed-out-settings', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, open: '&tab=general', has: ['Sign in', 'Settings', 'General', 'About', 'Open Beebeeb at login'], sidebar: ['Back to Sign in', 'General', 'About'], hifi: 'hifi-s4-fig6' },
  { name: 'session-ended', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }) }, has: ['Sign in again', 'Sync is paused until you sign in again. Your files stay in Finder.', 'Signed in before as sam@example.eu'], noSidebar: true },
]

const browser = await chromium.launch({ headless: true })
let passed = 0
const failures = []
const check = (ok, label, detail = '') => { if (ok) passed += 1; else failures.push(`${label}${detail ? ` (${detail})` : ''}`) }
const shots = []

try {
  for (const scheme of ['light', 'dark']) {
    for (const sc of SCENARIOS) {
      const sizes = [{ width: 920, height: 620, tag: '' }, ...(sc.min ? [{ width: 760, height: 520, tag: '-min' }] : [])]
      for (const size of sizes) {
        const label = `${sc.name}${size.tag} ${scheme}`
        const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, colorScheme: scheme, deviceScaleFactor: 1, locale: 'nl-NL' })
        const page = await context.newPage()
        const errors = []
        page.on('pageerror', (e) => errors.push(String(e)))
        page.on('console', (m) => { if (m.type() === 'error') errors.push(`console: ${m.text()}`) })
        await page.addInitScript(installBackend, { caps, fx: structuredClone({ ...base, ...(sc.fx ?? {}) }) })
        try {
          await page.goto(`${pathToFileURL(pagePath).href}?window=main&platform=macos${sc.open ?? ''}`)
          await page.waitForSelector('.mw-window', { timeout: 8000 })
          await page.waitForFunction(() => !document.body.innerText.includes('Loading…'), null, { timeout: 8000 })
          await page.waitForTimeout(400)
          if (sc.act) await sc.act(page)
          await page.waitForTimeout(500)
          const r = await page.evaluate(report)
          check(r.wide.length === 0, `${label}: nothing wider than its box (${r.rows} containers)`, r.wide.map((w) => `${w.el} ${w.scrollWidth} vs ${w.clientWidth}`).join('; '))
          check(r.rows >= 3, `${label}: the predicate found its containers`, `${r.rows}`)
          check(r.docScrollHeight <= r.innerHeight, `${label}: the document does not scroll`, `${r.docScrollHeight} vs ${r.innerHeight}`)
          for (const text of sc.has ?? []) check(r.text.includes(text), `${label}: shows "${text}"`)
          for (const text of sc.hasNot ?? []) check(!r.text.includes(text), `${label}: does not show "${text}"`)
          if (sc.noSidebar) check(r.sidebarWidth === null, `${label}: no sidebar while signed out`)
          else check(r.sidebarWidth === 214, `${label}: the sidebar is 214 pt`, `${r.sidebarWidth}`)
          if (!sc.noSidebar) check(r.toolbarHeight === 52, `${label}: the toolbar is 52 pt`, `${r.toolbarHeight}`)
          if (sc.bar) check(r.barHeight === 4, `${label}: the storage bar is drawn, 4 pt tall`, `${r.barHeight}`)
          if (sc.sidebar) check(JSON.stringify(r.sidebarButtons) === JSON.stringify(sc.sidebar), `${label}: the sidebar lists ${sc.sidebar.join(', ')}`, r.sidebarButtons.join(', '))
          if (sc.dialogs !== undefined) check(r.dialogs === sc.dialogs, `${label}: ${sc.dialogs} sheet`, `${r.dialogs}`)
          check(errors.length === 0, `${label}: no page errors`, errors.join(' | '))
          // The native traffic lights, drawn for the PNG only (the overlay title bar puts them over the sidebar).
          await page.evaluate(() => {
            const tl = document.createElement('div')
            tl.setAttribute('style', 'position:fixed;left:18px;top:18px;display:flex;gap:8px;z-index:2000')
            tl.innerHTML = ['#ff5f57', '#febc2e', '#28c840'].map((c) => `<div style="width:12px;height:12px;border-radius:50%;background:${c}"></div>`).join('')
            document.body.appendChild(tl)
          })
          const file = path.join(output, 'png', `${sc.name}${size.tag}-${scheme}.png`)
          await page.screenshot({ path: file })
          shots.push({ sc, scheme, size, file })
        } catch (error) {
          check(false, `${label}: drove the window to its state`, String(error).slice(0, 300))
        } finally {
          await context.close()
        }
      }
    }
  }
  // Side by side with the hi-fi's figures (each figure holds the light and dark copies of one screen).
  if (process.env.HIFI_PNG_DIR) {
    for (const sc of SCENARIOS.filter((s) => s.hifi)) {
      const theirs = path.join(process.env.HIFI_PNG_DIR, `${sc.hifi}.png`)
      if (!existsSync(theirs)) { check(false, `${sc.name}: hi-fi figure ${sc.hifi}.png exists`); continue }
      const light = path.join(output, 'png', `${sc.name}-light.png`)
      const dark = path.join(output, 'png', `${sc.name}-dark.png`)
      const html = path.join(output, 'png', `compare-${sc.name}.html`)
      await writeFile(html, `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#cfccc4;font:12px monospace;color:#222;padding:16px">
        <div style="margin-bottom:6px">hi-fi: design/hifi/macos-two-surface.html, ${sc.hifi}</div><img src="${pathToFileURL(theirs).href}" style="max-width:1180px">
        <div style="margin:16px 0 6px">this build, Chromium 1x, 920 x 620: light, then dark</div>
        <div style="display:flex;gap:16px"><img src="${pathToFileURL(light).href}"><img src="${pathToFileURL(dark).href}"></div></body>`)
      const page = await browser.newPage({ viewport: { width: 1900, height: 900 } })
      await page.goto(pathToFileURL(html).href)
      await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0))
      await page.screenshot({ path: path.join(output, 'png', `compare-${sc.name}.png`), fullPage: true })
      await page.close()
    }
  }
} finally {
  await browser.close()
}

console.log(`screenshots: ${shots.length}`)
if (failures.length > 0) {
  console.log(`RED: ${failures.length} failed, ${passed} passed`)
  for (const f of failures) console.log(`  FAIL ${f}`)
  process.exit(1)
}
console.log(`${passed} passed, 0 failed (the Beebeeb window in Chromium)`)
