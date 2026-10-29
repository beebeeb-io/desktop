// Real-browser check of the macOS Beebeeb window (task 1683 slices 1 and 2; spec docs/specs/2026-10-24-macos-two-surface-app.md
// §7, §8, §11; hi-fi design/hifi/macos-two-surface.html sections 2 to 5). Local evidence, not a CI gate (Playwright is
// not a desktop dependency); tests/macApp.test.tsx and tests/macSettingsLayout.test.ts pin the same rules in CI.
// Slice 2 adds the signed-out cards (section 2): sign in, the browser code, the email form, the second factor, Sign in
// again, set up this Mac, the Finder status and Home after it, and the account switch. The synthetic IPC has a working
// event bus (the browser code and the Finder screen are driven by events) and moves the session revision on every
// sign-in, so the real AccountSessionBoundary remounts the window exactly as it does on a Mac.
//
//   node tests/render-mac-app.mjs REPO OUTPUT
//   PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs   (as the other render-*.mjs)
//   HIFI_PNG_DIR=/path/to/hifi-figures               optional: side by side with the hi-fi's figures
//   RENDER_LOCALE=en-US                              the browser's language; sizes follow it, and so do the expected strings
//                                                    (hifi-sN-figK.png for sections 3 to 5, hifi-s2-figK-{light,dark}.png for section 2)
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

// The window writes sizes and counts in the language of the Mac ("84,3 GB" on a Dutch one, "84.3 GB" on an American one;
// storageFormat.ts). The browser is given ONE language, here, and every expected number below is written with that same
// language, so no check depends on which language that is (PR #140 review, P3: they spelled out a Dutch Mac's).
const LOCALE = process.env.RENDER_LOCALE || 'en-US'
const number = (value) => new Intl.NumberFormat(LOCALE, { minimumFractionDigits: 0, maximumFractionDigits: 1 }).format(value)
const gb = (value) => `${number(value)} GB`
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
// Slice 5: Security, General's account preferences, Storage details, About's channel / update / speed test.
const now = Date.now()
const hoursAgo = (n) => new Date(now - n * 3_600_000).toISOString()
const todayAt = (h, m) => { const d = new Date(); d.setHours(h, m, 0, 0); return d.toISOString() }
const account = {
  // The authenticator row reads the account's own `totp_enabled` (review P2-5), not a security-score factor.
  profile: { user_id: 'u1', email: 'sam@example.eu', email_verified: true, created_at: '2026-01-01T00:00:00Z', totp_enabled: true },
  score: { score: 4, max: 5, label: 'Good', factors: [{ key: 'email_verified', satisfied: true }, { key: 'two_factor_enabled', satisfied: true }] },
  sessions: { sessions: [
    { id: 's1', device_name: 'MacBook Pro', device_kind: 'desktop', country_code: 'NL', last_active_at: new Date(now).toISOString(), created_at: hoursAgo(900), is_current: true },
    { id: 's2', device_name: 'iPhone', device_kind: 'ios', country_code: 'NL', last_active_at: hoursAgo(2), created_at: hoursAgo(700), is_current: false },
  ] },
  devices: { devices: [{ id: 'd1', hostname: 'MacBook Pro', platform: 'macos', bb_version: '0.8.13', last_seen: new Date(now).toISOString(), created_at: new Date(new Date().getFullYear(), 8, 3, 9).toISOString(), session_count: 1 }] },
  feed: { events: [
    { id: 'e1', type: 'file.trash', subject: null, details: null, where: null, created_at: todayAt(9, 12) },
    { id: 'e2', type: 'file.downloaded', subject: null, details: null, where: null, created_at: todayAt(8, 40) },
  ], total: 2, page: 1 },
  prefs: { new_device_login: true, share_received: true, file_updated: false, storage_warning: true, backup_complete: false },
  breakdown: {
    by_category: [{ category: 'media', bytes: 60_000_000_000, file_count: 1200 }, { category: 'documents', bytes: 20_000_000_000, file_count: 3400 }, { category: 'other', bytes: 4_300_000_000, file_count: 90 }],
    largest_files: [{ file_id: 'f1', path: 'Photos/2026/Holiday in the mountains/IMG_0001.heic', size_bytes: 3_200_000_000, category: 'media' }, { file_id: 'f2', path: 'Investigations/Q3 ledger reconciliation.xlsx', size_bytes: 12_400_000, category: 'documents' }],
    total_files: 4690, total_bytes: 84_300_000_000,
  },
  speed: { latency: { min_ms: 18, avg_ms: 24, max_ms: 41 }, upload_bps: 4_400_000, download_bps: 11_750_000 },
}
const base = {
  ...account,
  update: { status: 'up_to_date', current_version: '0.8.12', channel: 'stable' },
  snapshot: snapshot(),
  finder: { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null },
  config: { upload_kbps_limit: 0, download_kbps_limit: 5000, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'system' },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-11-14T00:00:00Z', quota_bytes: 200_000_000_000, used_bytes: 84_300_000_000 },
  keptUnsent: null, kept: null,
  // Home's one-time hint (slice 6): dismissed in every scenario but the ones that draw it.
  hintSeen: true,
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
  // "Last checked 1 min ago" is relative to now, and now is the page's, not the script's: fixtures built when the script
  // started would read "2 min ago" for every scenario that a slow machine reaches a minute late.
  if (typeof fx.snapshot?.engine?.last_tick_ok_at === 'number') fx.snapshot.engine.last_tick_ok_at = Math.floor(Date.now() / 1000) - 70
  window.__fx = fx
  window.__titles = []
  // A working event bus: `listen` registers the page's callback, `__emit` is what Rust's `app.emit` does.
  const callbacks = new Map()
  const listeners = []
  let callbackIds = 0
  window.__emit = (event, payload) => {
    for (const l of listeners.filter((entry) => entry.event === event)) callbacks.get(l.handler)?.({ event, id: l.id, payload })
  }
  const signedIn = (f, unlocked) => {
    f.revision = (f.revision ?? 1) + 2 // a sign-in moves the revision; the boundary remounts the window on it
    f.snapshot = { ...f.snapshot, phase: unlocked ? 'synced' : 'locked', account: { ...f.snapshot.account, email: 'sam@example.eu', logged_in: true, vault_unlocked: unlocked, auth_expired: false } }
  }
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback: (callback) => { callbackIds += 1; callbacks.set(callbackIds, callback); return callbackIds },
    unregisterCallback() {},
    invoke: async (name, a) => {
      const f = window.__fx
      switch (name) {
        case 'plugin:event|listen': { listeners.push({ event: a.event, handler: a.handler, id: listeners.length + 1 }); return listeners.length }
        case 'plugin:event|unlisten': return null
        case 'plugin:window|set_title': window.__titles.push(a.value); return null
        case 'last_signed_in_email': return f.lastEmail ?? null
        case 'quit_beebeeb': return null
        case 'desktop_login':
        case 'desktop_login_2fa': {
          const outcome = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null }
          if (name === 'desktop_login' && f.login === 'twofa') return { ...outcome, requires_2fa: true }
          if (f.login === 'mismatch') return { ...outcome, account_mismatch: { pending_changes: 3, unsent: { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] } } }
          signedIn(f, false)
          return outcome
        }
        case 'desktop_unlock_with_recovery_phrase': {
          // The session is already here, so the revision does not move (an unlock moves nothing the boundary watches).
          f.snapshot = { ...f.snapshot, phase: 'synced', account: { ...f.snapshot.account, vault_unlocked: true } }
          return null
        }
        case 'start_browser_login': {
          window.__emit('browser-login', { attempt: a?.attemptId, phase: 'connecting' })
          window.__emit('browser-login', { attempt: a?.attemptId, phase: 'waiting', user_code: 'KQ7M-4TXP', verification_uri: 'https://app.beebeeb.io/cli-auth', expires_in: 300 })
          if (f.browser !== 'done') return new Promise(() => {})
          await new Promise((resolve) => setTimeout(resolve, 300))
          window.__emit('browser-login', { attempt: a?.attemptId, phase: 'authorized' })
          // C-S1 (task 1747 Part 1): the handoff's answer is the password path's LoginOutcome. Another account is an
          // outcome, not an event and not an error: nothing was installed, and the switch step shows the count.
          if (f.browserOutcome === 'mismatch') {
            return { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: { pending_changes: 3, unsent: { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] } } }
          }
          signedIn(f, true)
          window.__emit('browser-login', { attempt: a?.attemptId, phase: 'done', email: 'sam@example.eu' })
          return { requires_2fa: false, reauthenticated: false, vault_unlocked: true, key_replaced: false, account_mismatch: null }
        }
        case 'desktop_capabilities': return caps
        case 'desktop_platform': return 'macos'
        case 'app_version': return '0.8.12'
        case 'sync_status': return { session_revision: f.revision ?? 1, logged_in: f.snapshot.account.logged_in, vault_unlocked: f.snapshot.account.vault_unlocked, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: f.snapshot.account.auth_expired, engine_refusal: null }
        case 'popover_snapshot': return f.snapshot
        case 'account_subscription': return f.subscription
        case 'get_desktop_config': return f.config
        case 'desktop_config': return f.config
        case 'set_desktop_config': f.config = { ...f.config, ...a.config }; return null
        case 'autostart_enabled': return true
        case 'finder_setup_state': return f.finder
        case 'kept_unsent_folder': return f.keptUnsent
        case 'kept_unsynced_folder': return f.kept
        case 'get_menu_bar_hint_seen': return f.hintSeen
        case 'mark_menu_bar_hint_seen': f.hintSeen = true; return null
        case 'list_remote_tree': return [{ id: 'f1', name: 'Photos', is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: true, children: [] }]
        case 'account_region': return f.region
        case 'account_profile': return f.profile
        case 'account_profile_fresh': return f.profile
        case 'account_security_score': return f.score
        case 'account_session_list': return f.sessions
        case 'account_devices': return f.devices
        case 'account_activity_feed': return f.feed
        case 'account_notification_preferences': return { preferences: f.prefs }
        case 'account_update_notification_preferences': f.prefs = { ...f.prefs, ...a.update }; return { preferences: f.prefs }
        case 'account_storage_breakdown': return f.breakdown
        case 'run_speedtest': return f.speed
        case 'check_for_updates_now': return f.update
        case 'report_problem': return { path: '/Users/sam/Desktop/beebeeb-support.json', email_opened: true }
        case 'consume_main_navigation': { const nav = f.nav ?? null; f.nav = null; return nav }
        // The fix round after the /code-review of #143: Rust's sign-out (it moves the revision, as `clear_session_impl` does),
        // the unlock with no key, and the set-up step Rust keeps for the process.
        case 'clear_session': {
          f.snapshot = { ...f.snapshot, phase: 'signed_out', account: { ...f.snapshot.account, email: null, logged_in: false, vault_unlocked: false, auth_expired: false } }
          f.revision = (f.revision ?? 1) + 2
          if (f.clear === 'fail-after-removal') throw 'Sign-out stopped after Beebeeb was removed from Finder. Restart Beebeeb and sign in again.'
          return { warning: f.clear === 'warning' ? { code: 'finder_removal_unconfirmed', sentence: 'You are signed out, but Beebeeb could not confirm that it was removed from Finder. If it still shows there, restart Beebeeb and sign out again.' } : null }
        }
        case 'unlock_vault': { if (f.unlock === 'nokey') throw 'This Mac does not have your encryption keys yet. Restore them to continue.'; return null }
        case 'pending_setup': return f.pending ?? null
        // 1747 Part 2: the Keychain route. `route` is one answer; `routes` is a list read in order (the last one repeats), for a key that
        // vanishes between the window opening and the click.
        case 'vault_unlock_route': return f.routes ? (f.routes.length > 1 ? f.routes.shift() : f.routes[0]) : (f.route ?? 'keychain')
        case 'set_pending_setup': return null
        case 'sign_out_result_ack': return null
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
    titles: window.__titles,
    focused: document.activeElement && document.activeElement !== document.body ? (document.activeElement.getAttribute('aria-label') ?? document.activeElement.innerText.trim()) : null,
    codeFont: (() => { const c = document.querySelector('.mw-code'); return c ? getComputedStyle(c).fontFamily : null })(),
    cardWidth: (() => { const c = document.querySelector('.mw-scard'); return c ? Math.round(c.getBoundingClientRect().width) : null })(),
    // A card taller than the window must start at the top of the body (scrolling to the rest), never lose its top.
    cardTop: (() => { const c = document.querySelector('.mw-scard'); return c ? Math.round(c.getBoundingClientRect().top) : null })(),
    toolbarBottom: (() => { const t = document.querySelector('.mw-toolbar'); return t ? Math.round(t.getBoundingClientRect().bottom) : null })(),
    cardCentred: (() => {
      const c = document.querySelector('.mw-scard')
      if (!c) return null
      const r = c.getBoundingClientRect()
      return Math.abs((r.left + r.right) / 2 - innerWidth / 2) <= 1
    })(),
    sidebarButtons: sidebar ? [...sidebar.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.innerText.trim()) : [],
    text: document.body.innerText,
  }
}

const OUT = { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }
const gear = async (p) => { await p.getByRole('button', { name: 'Settings', exact: true }).click() }
const section = (label) => async (p) => { await gear(p); await p.getByRole('button', { name: label }).first().click() }

const SCENARIOS = [
  { name: 'home', bar: true, fx: { hintSeen: false }, has: ['Beebeeb lives in the menu bar', 'Click the b at the top of your screen to see what’s syncing. You can close this window; Beebeeb keeps running.', 'Got it', 'Home', 'End-to-end encrypted', 'Up to date', 'Last checked 1 min ago', 'Beebeeb in Finder', 'Your vault appears under Locations in Finder.', 'Open in Finder', 'Storage', `${gb(84.3)} of ${gb(200)}`, 'sam@example.eu', 'Basic plan'], hasNot: ['Needs your attention', 'control center', 'daemon'], hifi: 'hifi-s3-fig0' },
  { name: 'home-hint-dismissed', fx: { hintSeen: false }, act: async (p) => { await p.getByRole('button', { name: 'Got it' }).click(); await p.waitForFunction(() => !document.body.innerText.includes('Beebeeb lives in the menu bar')) }, has: ['Up to date', 'Beebeeb in Finder'], hasNot: ['Beebeeb lives in the menu bar', 'Got it'] },
  { name: 'home-kept', fx: { keptUnsent: KEPT_OURS, kept: KEPT_MACOS }, has: ['Needs your attention', KEPT_OURS, KEPT_MACOS, 'Show in Finder', 'Dismiss'] },
  { name: 'home-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, has: ['Your vault is locked', 'Unlock vault', 'Vault locked'], hasNot: ['Beebeeb in Finder'], hifi: 'hifi-s3-fig3' },
  { name: 'home-long-email', fx: { snapshot: snapshot({ account: { email: LONG_EMAIL } }) }, has: [LONG_EMAIL] },
  { name: 'settings-general', act: gear, has: ['Settings', 'Home', 'General', 'Open Beebeeb at login', 'Appearance', 'Follow macOS.', 'System', 'Tell me about', 'From your account', 'New device sign-in', 'We cannot see your files, but we know when a new device signs in.', 'Backup complete', 'When a scheduled backup finishes.'], sidebar: ['Back to Home', 'Account, sam@example.eu, Basic plan', 'General', 'Sync', 'Security', 'About'], min: true, hifi: 'hifi-s4-fig1' },
  { name: 'settings-account', act: section('Account, sam@example.eu, Basic plan'), has: ['Account', 'sam@example.eu', 'Basic plan · renews 14 Nov 2026', 'Storage details…', 'Vault is unlocked', 'Where your files are stored', 'Europe', 'Germany', 'Password, email and billing', 'Changed on beebeeb.io, in your browser.', 'Open beebeeb.io', 'Sign out of this Mac', 'Sign out…'], hasNot: ['Couldn’t load storage locations'], min: true, hifi: 'hifi-s4-fig2' },
  { name: 'storage-details', act: async (p) => { await section('Account, sam@example.eu, Basic plan')(p); await p.getByRole('button', { name: 'Storage details…' }).click(); await p.getByRole('dialog').waitFor(); await p.waitForFunction(() => !document.querySelector('[role="dialog"]').innerText.includes('Loading…')) }, has: ['Storage details', 'Where your storage goes, by type and by your largest files.', `${number(4690)} files · ${gb(84.3)}`, 'By type', 'Media', `${gb(60)} · ${number(1200)} files`, 'Documents', 'Other', 'Largest files', 'Photos/2026/Holiday in the mountains/IMG_0001.heic', `Media · ${gb(3.2)}`], hasNot: ['PC', 'daemon'], dialogs: 1, min: true },
  { name: 'settings-sync', act: section('Sync'), has: ['Beebeeb in Finder', 'Added', 'Repair…', 'Keep on this Mac', 'Speed'], min: true, hifi: 'hifi-s4-fig3' },
  { name: 'settings-security-no-totp', fx: { profile: { user_id: 'u1', email: 'sam@example.eu', email_verified: true, created_at: '2026-01-01T00:00:00Z', totp_enabled: false } }, act: section('Security'), has: ['Authenticator app (TOTP) · Off', 'Add a second factor so a stolen password alone can’t open your vault. Set it up on beebeeb.io.', 'Set up on beebeeb.io'], hasNot: ['Authenticator app (TOTP) · On', 'Manage or reset it'] },
  { name: 'settings-security', act: section('Security'), has: ['Security', 'Authenticator app (TOTP) · On', 'A second factor is required when you sign in. Manage or reset it on beebeeb.io.', 'Manage on beebeeb.io', 'Active sessions', 'MacBook Pro · This session', 'Active now', 'iPhone', 'Active 2 hours ago', 'Revoke', 'Sign out everywhere else', 'Sign out all other sessions', 'Devices', 'MacBook Pro · macOS · v0.8.13', 'Every device holding a copy of your vault key, and the sync sessions running on each.', 'Recent account activity', 'Moved an item to trash', 'Downloaded an item'], hasNot: ['Revoke a device', 'daemon'], sidebar: ['Back to Home', 'Account, sam@example.eu, Basic plan', 'General', 'Sync', 'Security', 'About'], min: true, hifi: 'hifi-s4-fig4' },
  { name: 'security-revoke', act: async (p) => { await section('Security')(p); await p.getByRole('button', { name: 'Revoke iPhone' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Sign out this session?', 'iPhone will need to sign in again.', 'Cancel', 'Sign out'], dialogs: 1 },
  { name: 'security-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, act: section('Security'), has: ['Your vault is locked', 'Unlock it to see your sessions, devices and recent activity.'], hasNot: ['Active sessions'] },
  { name: 'settings-about', act: section('About'), has: ['Beebeeb for Mac', 'Version 0.8.12', 'Check for updates', 'Beebeeb checks for updates at launch and every 4 hours.', 'Release channel', 'Stable', 'Regular releases after validation.', 'Get help', 'Test connection speed', 'Run test', 'Export a support bundle'], hasNot: ['Restart to update', 'parity', 'CLI'], min: true, hifi: 'hifi-s4-fig5' },
  { name: 'about-update-available', fx: { update: { status: 'update_available', current_version: '0.8.12', channel: 'stable', version: '0.8.13', body: '', release_notes_url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.13' } }, act: async (p) => { await section('About')(p); await p.getByRole('button', { name: 'Check for updates' }).click(); await p.getByRole('button', { name: 'Download update' }).waitFor() }, has: ['Version 0.8.13 is available. Drag Beebeeb into Applications to replace this version.', 'Download update'], hasNot: ['Restart to update', 'Check for updates'] },
  { name: 'about-downgrade', fx: { update: { status: 'downgrade_available', current_version: '0.8.12', current_channel: 'beta', channel: 'stable', version: '0.8.11', body: 'Fixes the sync stall.\n\nNothing else changed.', release_notes_url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.11' }, config: { ...base.config, release_channel: 'stable' } }, act: async (p) => { await section('About')(p); await p.getByRole('button', { name: 'Check for updates' }).click(); await p.getByRole('button', { name: 'Release notes' }).waitFor() }, has: ['The Stable channel is at version 0.8.11', 'This Mac runs 0.8.12, from the Beta channel. Read the release notes before you go back to an older version.', 'Release notes', 'View release'], hasNot: ['Downgrade to'], min: true },
  { name: 'about-release-notes', fx: { update: { status: 'downgrade_available', current_version: '0.8.12', current_channel: 'beta', channel: 'stable', version: '0.8.11', body: 'Fixes the sync stall.\n\nNothing else changed.', release_notes_url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.11' } }, act: async (p) => { await section('About')(p); await p.getByRole('button', { name: 'Check for updates' }).click(); await p.getByRole('button', { name: 'Release notes' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Beebeeb 0.8.11', 'Fixes the sync stall.', 'Nothing else changed.'], dialogs: 1 },
  { name: 'about-speed-test', act: async (p) => { await section('About')(p); await p.getByRole('button', { name: 'Run test' }).click(); await p.getByText('Latency 24 ms (18 to 41)').waitFor() }, has: ['Latency 24 ms (18 to 41) · Upload 35.2 Mbps · Download 94.0 Mbps'] },
  { name: 'signout-confirm', act: async (p) => { await section('Account, sam@example.eu, Basic plan')(p); await p.getByRole('button', { name: 'Sign out…' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Sign out of this Mac?', 'Sync stops until you sign in again.', 'Cancel', 'Sign out'], dialogs: 1, hifi: 'hifi-s5-fig0' },
  // ── Signed out (slice 2, hi-fi section 2): one centred card at a time, no sidebar ──
  { name: 'signed-out', fx: OUT, has: ['Sign in to Beebeeb', 'Sign in with browser', 'Use email and password', 'New to Beebeeb?', 'Create an account on beebeeb.io', 'Quit Beebeeb'], hasNot: ['Home', 'Settings', 'daemon'], noSidebar: true, card: 380, title: 'Sign in to Beebeeb', hifiFig: 0, min: true },
  { name: 'signin-browser-waiting', fx: OUT, act: async (p) => { await p.getByRole('button', { name: 'Sign in with browser' }).click(); await p.getByText('KQ7M-4TXP').waitFor() }, has: ['Type this code on the page we just opened in your browser.', 'KQ7M-4TXP', 'Browser didn’t open? Visit', 'app.beebeeb.io/cli-auth', 'Copy link', 'Waiting for browser…', 'Use email and password'], hasNot: ['https://'], noSidebar: true, card: 380, codeMono: true, hifiFig: 1 },
  { name: 'signin-password', fx: { ...OUT, lastEmail: 'sam@example.eu' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').waitFor() }, has: ['Welcome back', 'Sign in to unlock your encrypted vault.', 'Email', 'Password', 'Sign in with browser instead'], noSidebar: true, card: 380, title: 'Welcome back', hifiFig: 2 },
  { name: 'signin-2fa', fx: { ...OUT, lastEmail: 'sam@example.eu', login: 'twofa' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Two-factor authentication').waitFor() }, has: ['Two-factor authentication', 'Enter the 6-digit code from your authenticator app.', 'Authentication code', 'Verify', 'Use backup code'], noSidebar: true, card: 380 },
  { name: 'session-ended', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }) }, has: ['Sign in again', 'Sync is paused until you sign in again. Your files stay in Finder.', 'Signed in before as sam@example.eu', 'Sign in with browser', 'Use email and password', 'Quit Beebeeb'], hasNot: ['Create an account'], noSidebar: true, card: 380, title: 'Sign in again', hifiFig: 3 },
  { name: 'set-up-this-mac', fx: { ...OUT, lastEmail: 'sam@example.eu', login: 'fresh' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Set up this Mac').waitFor() }, has: ['Set up this Mac', 'This Mac does not have your encryption keys yet. Restore them to continue.', 'Recovery phrase', 'Paste all 12 words into any box or type them one by one.', 'Unlock vault'], noSidebar: true, card: 440, title: 'Set up this Mac', hifiFig: 4, min: true },
  { name: 'finder-adding', fx: { ...OUT, browser: 'done', finder: { setup: 'adding', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 3, last_failure: null } }, act: async (p) => { await p.getByRole('button', { name: 'Sign in with browser' }).click(); await p.getByText('Adding Beebeeb to Finder…').waitFor() }, has: ['Beebeeb in Finder', 'Beebeeb adds itself to Finder after sign-in.', 'Adding Beebeeb to Finder…', 'Continue to Home'], noSidebar: true, card: 440, title: 'Beebeeb in Finder', hifiFig: 5 },
  { name: 'finder-failed', fx: { ...OUT, browser: 'done', finder: { setup: 'user_disabled', reason: 'user_disabled', launch_location: 'applications', attempt: 1, max_attempts: 3, last_failure: null } }, act: async (p) => { await p.getByRole('button', { name: 'Sign in with browser' }).click(); await p.getByText('Beebeeb is turned off in System Settings.').waitFor() }, has: ['Beebeeb in Finder', 'Beebeeb is turned off in System Settings.', 'Open System Settings', 'Continue to Home'], noSidebar: true, card: 440 },
  // The Finder status moves the window on to Home when the reconciler says Ready: the same window, no close, no new window.
  { name: 'finder-ready-home', fx: { ...OUT, browser: 'done', finder: { setup: 'adding', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 3, last_failure: null } }, act: async (p) => {
    await p.getByRole('button', { name: 'Sign in with browser' }).click()
    await p.getByText('Adding Beebeeb to Finder…').waitFor()
    await p.evaluate(() => { const ready = { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 3, last_failure: null }; window.__fx.finder = ready; window.__emit('finder-setup-changed', ready) })
    await p.getByText('End-to-end encrypted').waitFor()
  }, has: ['Home', 'End-to-end encrypted', 'Beebeeb in Finder', 'Your vault appears under Locations in Finder.', 'Open in Finder', 'sam@example.eu'], hasNot: ['Adding Beebeeb to Finder…', 'Continue', 'Control center'], title: 'Beebeeb' },
  { name: 'account-switch', fx: { ...OUT, lastEmail: 'sam@example.eu', login: 'mismatch' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Switch accounts with changes not sent?').waitFor() }, has: ['Switch accounts with changes not sent?', 'This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.', 'Cancel', 'Sign out and switch'], noSidebar: true, card: 460, focus: 'Cancel', hifiFig: 6, min: true },
  // ── The fix round after the /code-review of #143 (real AccountSessionBoundary, in Chromium) ──
  // P2-1: a sign-out that happened with an unconfirmed Finder removal is HELD, so the signed-out card shows it after the
  // remount the sign-out causes (a toast did not survive it). The window stays on the card: no Settings, no sidebar.
  { name: 'switch-signout-warning', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }), lastEmail: 'sam@example.eu', login: 'mismatch', clear: 'warning' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Switch accounts with changes not sent?').waitFor(); await p.getByRole('button', { name: 'Sign out and switch' }).click(); await p.getByText('could not confirm that it was removed from Finder').waitFor() }, has: ['You are signed out, but Beebeeb could not confirm that it was removed from Finder.', 'Welcome back'], hasNot: ['Switch accounts with changes not sent?', 'Settings'], noSidebar: true, card: 380 },
  { name: 'switch-signout-failed-after-removal', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }), lastEmail: 'sam@example.eu', login: 'mismatch', clear: 'fail-after-removal' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Switch accounts with changes not sent?').waitFor(); await p.getByRole('button', { name: 'Sign out and switch' }).click(); await p.getByText('Sign-out stopped after Beebeeb was removed from Finder.').waitFor() }, has: ['Sign-out stopped after Beebeeb was removed from Finder. Restart Beebeeb and sign in again.'], hasNot: ['Switch accounts with changes not sent?'], noSidebar: true, card: 380 },
  // P2-2: the account signs out another way (the app menu) while the switch card is up. The revision moves; the card must
  // not come back after the remount, and the address typed for the next sign-in stays (the email form opens with it).
  { name: 'switch-card-then-menu-sign-out', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }), lastEmail: 'sam@example.eu', login: 'mismatch' }, act: async (p) => { await p.getByRole('button', { name: 'Use email and password' }).click(); await p.getByLabel('Password').fill('correct horse'); await p.getByRole('button', { name: 'Sign in', exact: true }).click(); await p.getByText('Switch accounts with changes not sent?').waitFor(); await p.evaluate(() => { const f = window.__fx; f.snapshot = { ...f.snapshot, phase: 'signed_out', account: { ...f.snapshot.account, email: null, logged_in: false, vault_unlocked: false, auth_expired: false } }; f.revision = (f.revision ?? 1) + 2 }); await p.getByText('Welcome back').waitFor() }, has: ['Welcome back', 'Sign in with browser instead'], hasNot: ['Switch accounts with changes not sent?', 'Sign out and switch'], noSidebar: true, card: 380 },
  // P2-3 b: the Keychain holds a token and no key. The unlock says the approved sentence, and offers the phrase.
  { name: 'locked-without-key', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), unlock: 'nokey' }, act: async (p) => { await p.getByRole('button', { name: 'Unlock vault' }).click(); await p.getByRole('button', { name: 'Use your recovery phrase' }).waitFor() }, has: ['Your vault is locked', 'This Mac does not have your encryption keys yet. Restore them to continue.', 'Use your recovery phrase'], hasNot: ['secret not found', 'Sign in before unlocking'] },
  { name: 'locked-without-key-phrase', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), unlock: 'nokey' }, act: async (p) => { await p.getByRole('button', { name: 'Unlock vault' }).click(); await p.getByRole('button', { name: 'Use your recovery phrase' }).click(); await p.getByText('Set up this Mac').waitFor() }, has: ['Set up this Mac', 'Recovery phrase', 'Unlock vault'], noSidebar: true, card: 440, title: 'Set up this Mac' },
  // 1747 Part 2 (plan D4, item 14): the interrupted first run. A session in the Keychain, no key anywhere, nothing pending in Rust (the app was
  // quit): the window opens on Set up this Mac by state, with no click. With the key in the Keychain it is Home's locked card.
  { name: 'locked-no-key-opens-set-up', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), route: 'recovery_phrase' }, has: ['Set up this Mac', 'This Mac does not have your encryption keys yet. Restore them to continue.', 'Recovery phrase'], hasNot: ['Your vault is locked', 'Vault locked'], noSidebar: true, card: 440, title: 'Set up this Mac' },
  { name: 'locked-with-key-opens-home', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), route: 'keychain' }, has: ['Your vault is locked', 'Unlock vault', 'Vault locked'], hasNot: ['Set up this Mac', 'Recovery phrase'] },
  // The click asks again: the key vanished after the window opened, so “Unlock vault” goes to Set up this Mac and never to a Keychain error.
  { name: 'locked-key-vanished-before-the-click', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), routes: ['keychain', 'recovery_phrase'], unlock: 'nokey' }, act: async (p) => { await p.getByRole('button', { name: 'Unlock vault' }).click(); await p.getByText('Set up this Mac').waitFor() }, has: ['Set up this Mac', 'Recovery phrase'], hasNot: ['Your vault is locked', 'secret not found'], noSidebar: true, card: 440, title: 'Set up this Mac' },
  // P2-3 a: a window reopened (⌘W destroyed the earlier one) while a password sign-in was waiting for the recovery phrase.
  { name: 'reopened-pending-setup', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }), pending: { key_replaced: false } }, has: ['Set up this Mac', 'This Mac does not have your encryption keys yet. Restore them to continue.'], hasNot: ['Your vault is locked'], noSidebar: true, card: 440, title: 'Set up this Mac' },
  // P3-1: `open_reauth_window` asks for Sign in again whatever the session says.
  { name: 'sign-in-again-asked', fx: { nav: { to: 'reauth' } }, has: ['Sign in again', 'Beebeeb couldn’t send some changes with this sign-in. Sign in again to retry them.', 'Signed in before as sam@example.eu', 'Back to Home'], hasNot: ['Sync is paused'], noSidebar: true, card: 380, title: 'Sign in again' },
  // C-S1 (task 1747 Part 1): a different account through the browser reaches the switch step through the handoff's returned
  // outcome, with its count; no raw code is on the page.
  { name: 'browser-account-switch', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }), browser: 'done', browserOutcome: 'mismatch' }, act: async (p) => { await p.getByRole('button', { name: 'Sign in with browser' }).click(); await p.getByText('Switch accounts with changes not sent?').waitFor() }, has: ['Switch accounts with changes not sent?', 'Cancel', 'Sign out and switch'], hasNot: ['account_mismatch', 'Sign-in failed'], noSidebar: true, card: 460, focus: 'Cancel' },
  { name: 'signed-out-settings', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, open: '&tab=general', has: ['Sign in', 'Settings', 'General', 'About', 'Open Beebeeb at login', 'Appearance', 'Tell me about'], hasNot: ['From your account'], sidebar: ['Back to Sign in', 'General', 'About'], hifi: 'hifi-s4-fig6' },
  { name: 'signed-out-about', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, open: '&tab=about', has: ['Beebeeb for Mac', 'Release channel'], hasNot: ['Test connection speed'] },
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
        const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, colorScheme: scheme, deviceScaleFactor: 1, locale: LOCALE })
        const page = await context.newPage()
        const errors = []
        page.on('pageerror', (e) => errors.push(String(e)))
        page.on('console', (m) => { if (m.type() === 'error') errors.push(`console: ${m.text()}`) })
        // "Last checked 1 min ago" must hold at every point of a long run: the check time is relative to now, per scenario.
        const fixture = structuredClone({ ...base, ...(sc.fx ?? {}) })
        fixture.snapshot.engine.last_tick_ok_at = Math.floor(Date.now() / 1000) - 70
        await page.addInitScript(installBackend, { caps, fx: fixture })
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
          // Slice 2: the signed-out cards (hi-fi section 2).
          if (sc.card) {
            check(r.cardWidth === sc.card, `${label}: the card is ${sc.card} pt wide`, `${r.cardWidth}`)
            check(r.cardCentred === true, `${label}: the card is centred in the window`)
            check(r.cardTop >= r.toolbarBottom, `${label}: the card's top is reachable, below the toolbar (a taller-than-window card scrolls from its top)`, `top ${r.cardTop}, toolbar ends ${r.toolbarBottom}`)
          }
          if (sc.title) check(r.titles.at(-1) === sc.title, `${label}: the window is titled "${sc.title}"`, r.titles.join(' | '))
          if (sc.codeMono) check(/JetBrains Mono|monospace/.test(r.codeFont ?? ''), `${label}: the device code is mono`, `${r.codeFont}`)
          if (sc.focus) check(r.focused === sc.focus, `${label}: focus is on ${sc.focus}`, `${r.focused}`)
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
  // Section 2's figures are one screen in one theme each (hifi-s2-figK-{light,dark}.png): compare both themes.
  if (process.env.HIFI_PNG_DIR) {
    for (const sc of SCENARIOS.filter((s) => s.hifiFig !== undefined)) {
      const theirs = ['light', 'dark'].map((scheme) => path.join(process.env.HIFI_PNG_DIR, `hifi-s2-fig${sc.hifiFig}-${scheme}.png`))
      if (!theirs.every((file) => existsSync(file))) { check(false, `${sc.name}: hi-fi figures hifi-s2-fig${sc.hifiFig}-{light,dark}.png exist`); continue }
      const ours = ['light', 'dark'].map((scheme) => path.join(output, 'png', `${sc.name}-${scheme}.png`))
      if (!ours.every((file) => existsSync(file))) { check(false, `${sc.name}: this build's screenshots exist to compare`); continue }
      const html = path.join(output, 'png', `compare-${sc.name}.html`)
      await writeFile(html, `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#cfccc4;font:12px monospace;color:#222;padding:16px">
        <div style="margin-bottom:6px">hi-fi: design/hifi/macos-two-surface.html, section 2, figure ${sc.hifiFig}: light, then dark</div>
        <div style="display:flex;gap:16px">${theirs.map((f) => `<img src="${pathToFileURL(f).href}" style="width:920px">`).join('')}</div>
        <div style="margin:16px 0 6px">this build, Chromium 1x, 920 x 620: light, then dark</div>
        <div style="display:flex;gap:16px">${ours.map((f) => `<img src="${pathToFileURL(f).href}">`).join('')}</div></body>`)
      const page = await browser.newPage({ viewport: { width: 1900, height: 1200 } })
      await page.goto(pathToFileURL(html).href)
      await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0))
      await page.screenshot({ path: path.join(output, 'png', `compare-${sc.name}.png`), fullPage: true })
      await page.close()
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
