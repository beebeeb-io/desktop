#!/usr/bin/env bun
// The desktop v2 action map audit (batch M1). For every action on the board (frames.json → Windows.frames[].notes.Actions)
// it opens the source frame in the running dev server, finds the control the action names, triggers it and records what
// happened: ok, missing-control, no-effect, wrong-target, command-unsupported or not-automatable.
//
// Usage (the dev server must be running):
//   bun scripts/v2-map-audit/audit.mjs --base http://localhost:5221 --out <dir> [--only DW-0401,DW-0402] [--plant] [--workers 4]
//
// How a screen is recognised: every frame is rendered once through the frame registry (?frame=DW-xxxx) and reduced to a
// signature: its headings, dialog names, menu items, button and link names, selected/pressed/current controls and its
// data-screen / data-frame markers. After an action, the live page's signature is compared with the target frame's.
//
// How a source frame is opened: the sections' own routes (?section=…, ?sub=…, the tray and first-run windows) are the
// live roots. A frame reached by an `ok` action from a live frame is opened the same way (the root, then the recorded
// steps), so most rows run in the live app. A frame no live path reaches yet falls back to its registry renderer, which
// mounts the same section components at an initial state; the row says which (`entry`).
//
// --plant replaces one action's control name with a label that exists nowhere, and the run fails unless that row comes
// back missing-control: the check proves it can fail before its oks are trusted.

import { chromium } from 'playwright'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith('--')) acc.push([a.slice(2), all[i + 1] && !all[i + 1].startsWith('--') ? all[i + 1] : 'true'])
    return acc
  }, []),
)
const WORKSPACE = process.env.BB_WORKSPACE ?? join(process.env.HOME ?? '', 'code/beebeeb.io')
const BASE = args.base ?? 'http://localhost:5221'
const BOARD = args.board ?? join(WORKSPACE, 'design/desktop-v2/frames.json')
const BATCHES = args.batches ?? join(WORKSPACE, 'design/desktop-v2/BATCHES.md')
const OUT = args.out ?? join(WORKSPACE, '.claude/tasks/verification-evidence/desktop-v2/m1')
const ONLY = args.only ? new Set(args.only.split(',')) : null
const WORKERS = Number(args.workers ?? 4)
const PLANT = args.plant === 'true'
const SETTLE_MS = 350
const BACKGROUND_COMMANDS = new Set(['sync_status', 'desktop_config', 'get_config', 'popover_snapshot'])
const EXCLUDED_SECTIONS = new Set(['Mail Backup on Windows (proposal)'])

const overrides = JSON.parse(readFileSync(join(HERE, 'controls.json'), 'utf8'))
const owners = JSON.parse(readFileSync(join(HERE, 'owners.json'), 'utf8'))

// ---------- the board ----------
// Some of the board's text went through cp1252 once ("Shareâ€¦" for "Share…"): undo that where it decodes cleanly.
const CP1252 = { 0x20ac: 0x80, 0x201a: 0x82, 0x192: 0x83, 0x201e: 0x84, 0x2026: 0x85, 0x2020: 0x86, 0x2021: 0x87, 0x2c6: 0x88, 0x2030: 0x89, 0x160: 0x8a, 0x2039: 0x8b, 0x152: 0x8c, 0x17d: 0x8e, 0x2018: 0x91, 0x2019: 0x92, 0x201c: 0x93, 0x201d: 0x94, 0x2022: 0x95, 0x2013: 0x96, 0x2014: 0x97, 0x2dc: 0x98, 0x2122: 0x99, 0x161: 0x9a, 0x203a: 0x9b, 0x153: 0x9c, 0x17e: 0x9e, 0x178: 0x9f }
function unmangle(s) {
  if (typeof s !== 'string' || !/[\u00c2-\u00f4][\u0080-\u2122]/.test(s)) return s
  return s.replace(/[\u00c2-\u00f4][\u0080-\u00bf\u0152-\u2122]{1,3}/g, (run) => {
    const bytes = [...run].map((c) => CP1252[c.codePointAt(0)] ?? c.codePointAt(0))
    if (bytes.some((b) => b > 0xff)) return run
    try {
      return new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(bytes))
    } catch {
      return run
    }
  })
}
const board = JSON.parse(readFileSync(BOARD, 'utf8'), (_k, v) => unmangle(v)).Windows.frames.filter((f) => !EXCLUDED_SECTIONS.has(f.section))
const FRAME_IDS = new Set(board.map((f) => f.id))
const batchOf = {}
{
  let current = null
  for (const line of readFileSync(BATCHES, 'utf8').split('\n')) {
    const h = line.match(/^### (B\d+[a-z]?)\b/)
    if (h) current = h[1]
    if (current && /Frames/.test(line)) for (const id of line.match(/DW-[0-9A-Z]+[a-z]?/g) ?? []) batchOf[id] ??= current
  }
}
const actions = []
for (const f of board) {
  ;(f.notes.Actions ?? []).forEach((a, i) => {
    actions.push({ key: `${f.id}#${i}`, source: f.id, index: i, on: a.on, does: a.does, to: (a.to ?? []).filter((t) => FRAME_IDS.has(t)) })
  })
}
if (PLANT) {
  const victim = actions.find((a) => a.source === 'DW-0401' && /^New$/.test(a.on)) ?? actions[0]
  victim.on = 'Click "Zq planted label that exists nowhere"'
  victim.planted = true
}

// Which window a frame is drawn in: an action whose target is in another window is Tauri's to open.
const SECTION_WINDOW = {
  'The notification area: the b and its flyout': 'the tray flyout', 'The b, its menu, and the app menus': 'the tray flyout',
  'File Explorer': 'File Explorer', 'Install Beebeeb on Windows': 'the installer',
  'First run': 'sign-in', 'Sign in': 'sign-in', 'Continue as': 'sign-in', 'Another device': 'sign-in', 'Email and password': 'sign-in',
  'Two-factor and passkeys': 'sign-in', 'Create an account': 'sign-in', 'Forgot password': 'sign-in', 'Before Files: locked, no plan yet': 'sign-in',
  'First run on Windows': 'first run', 'The first-time guide': 'first run',
}
const SECTION_OF = Object.fromEntries(board.map((f) => [f.id, f.section]))
const windowOf = (id) => SECTION_WINDOW[SECTION_OF[id]] ?? 'the main window'
const url = (q) => `${BASE}/?${q}`
const frameUrl = (id) => url(`window=main-app&platform=windows&frame=${id}`)

// ---------- the signature of a screen ----------
async function signature(page) {
  return page.evaluate(() => {
    const out = new Set()
    const vis = (e) => (e.checkVisibility ? e.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) : true)
    // Names and numbers are the data, not the screen: the live source and the frames' fixtures hold different files.
    const clean = (s) => (s ?? '').replace(/\s+/g, ' ').trim().replace(/[^\s"“(]+\.(pdf|docx?|xlsx?|pptx?|jpe?g|png|heic|mov|mp4|md|txt|svg|m4a|zip|csv|key|pages|numbers)\b/gi, '<file>').replace(/\d+([.,:]\d+)*/g, '#').slice(0, 60)
    const labelOf = (e) => {
      const by = e.getAttribute('aria-labelledby')
      if (by) return clean(by.split(' ').map((id) => document.getElementById(id)?.textContent ?? '').join(' '))
      return clean(e.getAttribute('aria-label') || e.getAttribute('title') || e.innerText || e.textContent)
    }
    const add = (kind, sel, short) => {
      for (const e of document.querySelectorAll(sel)) {
        if (!vis(e)) continue
        const n = labelOf(e)
        if (n && (!short || n.length <= (short === true ? 40 : short))) out.add(`${kind}:${n}`)
      }
    }
    add('h', 'h1,h2,h3,h4,[role=heading]')
    add('d', '[role=dialog],[role=alertdialog],dialog[open]')
    add('m', '[role=menuitem],[role=menuitemradio],[role=menuitemcheckbox]')
    add('b', 'button,a[href],[role=button],[role=link],[role=tab]', 24)
    add('sel', '[aria-selected=true],[aria-current]:not([aria-current=false]),[aria-pressed=true],[aria-checked=true],[aria-expanded=true]', true)
    add('st', '[role=status],[role=alert]')
    add('in', 'input[placeholder],textarea[placeholder]')
    for (const e of document.querySelectorAll('[data-screen],[data-frame]')) {
      if (vis(e)) out.add(`s:${e.getAttribute('data-screen') ?? e.getAttribute('data-frame')}`)
    }
    if (document.querySelector('[role=menu]')) out.add('menu')
    return [...out]
  })
}

const cover = (sig, state) => (sig.length ? sig.filter((t) => state.has(t)).length / sig.length : 0)
const same = (a, b) => a.size === b.size && [...a].every((t) => b.has(t))

/** Waits until the screen stops changing (lazy sections, fixtures that answer a tick later): two equal reads in a row. */
async function settle(page, max = 4000) {
  let last = ''
  const until = Date.now() + max
  await page.waitForTimeout(150)
  while (Date.now() < until) {
    const now = JSON.stringify(await signature(page).catch(() => []))
    if (now === last && now !== '[]') return
    last = now
    await page.waitForTimeout(SETTLE_MS)
  }
}

async function load(page, target) {
  await page.goto(target, { waitUntil: 'load' })
  await page.waitForFunction(() => (document.getElementById('root')?.innerText ?? '').trim().length > 0, null, { timeout: 8000 }).catch(() => {})
  await settle(page)
}

// ---------- reading the `on` text ----------
const KEY = String.raw`(?:(?:Ctrl|Alt|Shift|Win)\+)*(?:F\d{1,2}|Esc|Escape|Enter|Return|Space|Delete|Del|Tab|Backspace|Home|End|PageUp|PageDown|Page Up|Page Down|Up|Down|Left|Right|[↑↓←→]|Arrow ?(?:Up|Down|Left|Right)|[A-Z0-9,.+\-/\\])`
const KEY_ONLY = new RegExp(`^(?:${KEY})(?:\\s*(?:,|/|or)\\s*(?:${KEY}))*$`)
const OS_DRAWN = /\b(File Explorer|Explorer|installer|taskbar|notification area|system tray|Start menu|toast|UAC|SmartScreen|Windows Hello|Windows Security|desktop shortcut|Alt\+Tab|Win\+|Windows (?:shows|draws|asks)|the b\b|Jump List|another app|the OS|outside the window|resize|drag the window|Snap|title bar|caption bar buttons|minimi[sz]e|maximi[sz]e|the window's close)/i
const DRAG = /\bdrag/i
const HOVER = /^(hover|point at|pointer over)\b/i
const EVENT = /^(approved|denied|countdown|when\b|after\b|once\b|if\b|the server|a server|wrong\b|correct\b|code arrives|.*\breaches\b|times? out|expires|.*\b(finishes|completes|succeeds|fails)\b|back from|coming back|leave the page|hold (ctrl|shift|alt)|pinch|hover|point at|a minute later|later\b|on success|on error|no answer|the upload|the download|sync (finishes|stops))/i

function keyName(k) {
  const map = { Esc: 'Escape', Del: 'Delete', Return: 'Enter', Space: ' ', '↑': 'ArrowUp', '↓': 'ArrowDown', '←': 'ArrowLeft', '→': 'ArrowRight', '†“': 'ArrowDown', '†‘': 'ArrowUp', Up: 'ArrowUp', Down: 'ArrowDown', Left: 'ArrowLeft', Right: 'ArrowRight', 'Page Up': 'PageUp', 'Page Down': 'PageDown', Win: 'Meta', Ctrl: 'Control' }
  return k
    .split('+')
    .map((p) => p.trim())
    .map((p) => map[p] ?? p.replace(/^Arrow ?/, 'Arrow'))
    .map((p) => (p === ' ' ? 'Space' : p))
    .join('+')
}

function nameFrom(text) {
  const q = text.match(/[“"‘']([^”"’']{1,60})[”"’']/)
  if (q) return q[1].replace(/…$/, '').trim()
  return text
    .replace(/^(?:click|tap|press|choose|select|pick|open|use|turn|toggle|switch|type in|type|enter|check|uncheck|the|a|an|on|in|its|their)\s+/gi, '')
    .replace(/^(?:click|tap|press|choose|select|pick|the|a|an|its)\s+/gi, '')
    .replace(/\s+(?:button|link|row|switch|toggle|menu item|item|tab|icon|field|entry|chip|control|arrow|option)s?$/i, '')
    .replace(/\s*\([^)]*\)$/, '')
    .replace(/…$/, '')
    .trim()
}

/** The plan for one action: one or more alternatives, tried in order. */
function planFor(action) {
  const o = overrides[action.key]
  if (o) return Array.isArray(o) ? o : [o]
  const on = action.on.trim()
  if (OS_DRAWN.test(on) && !/[“"]/.test(on)) return [{ notAutomatable: `drawn or handled by Windows: "${on}"` }]
  if (DRAG.test(on) || /^drop\b/i.test(on)) return [{ notAutomatable: `a drag gesture: "${on}"` }]
  if (EVENT.test(on)) return [{ notAutomatable: `an event, not a control: "${on}"` }]
  const menu = on.match(/^right-click(?: (.+?))?\s*›\s*(.+)$/i)
  if (menu) return [{ verb: 'right-click', name: menu[1] && !/^(a|an|the) /i.test(menu[1]) ? nameFrom(menu[1]) : null, next: { verb: 'click', name: nameFrom(menu[2]) } }]
  if (/^type\b/i.test(on)) return [{ verb: 'fill', selector: 'input[type=search],input[type=text],[role=searchbox],[role=combobox],input:not([type])', value: 'contract' }]
  const kind = on.match(/^(?:a|an|any|one|each) (switch|toggle|box|checkbox|pill|filter|chip|tab|choice|option)\b/i)
  if (kind) return [{ verb: 'click', selector: /switch|toggle|box|checkbox/i.test(kind[1]) ? '[role=switch],[role=checkbox],input[type=checkbox]' : '[aria-pressed],[role=tab],[role=radio],[role=option]' }]
  const alts = on.split(/\s+or\s+|\s+\/\s+|(?<!\+),\s+(?=(?:Ctrl|Alt|Shift|F\d|Esc|Enter|Space|Delete|Tab|click|right-click|double-click))/i).map((s) => s.trim()).filter(Boolean)
  const plans = []
  for (const alt of alts) {
    if (KEY_ONLY.test(alt)) {
      for (const k of alt.split(/(?<!\+)\s*(?:,|\/|\bor\b)\s*/)) plans.push({ key: keyName(k) })
      continue
    }
    const m = alt.match(/^(right-click|double-click|ctrl\+click|shift\+click|hover|click|tap)\s*(.*)$/i)
    let verb = m ? m[1].toLowerCase() : 'click'
    let rest = m ? m[2] : alt
    if (HOVER.test(alt)) { verb = 'hover'; rest = alt.replace(HOVER, '') }
    const name = nameFrom(rest)
    const generic = /^(?:a|an|any|one|each)\s+(?:[\w-]+\s+)?(row|file|folder|item|photo|card|tile|result|entry|line|thumbnail|device|session|passkey|version|share|person|request|search)s?\b/i.test(rest) || /^(rows?|files?|items?)$/i.test(rest) || name === ''
    plans.push({ verb, name: generic ? null : name, generic: generic ? rest : null })
  }
  return plans
}

const ROLES = ['button', 'link', 'menuitem', 'menuitemradio', 'menuitemcheckbox', 'tab', 'switch', 'checkbox', 'radio', 'option', 'treeitem', 'row', 'gridcell', 'textbox', 'combobox', 'searchbox', 'listitem']
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

async function firstVisible(locator) {
  const n = Math.min(await locator.count().catch(() => 0), 12)
  for (let i = 0; i < n; i++) {
    const l = locator.nth(i)
    if (await l.isVisible().catch(() => false)) return l
  }
  return null
}

async function findControl(page, p) {
  for (let tries = 0; tries < 3; tries++) {
    const found = await findOnce(page, p)
    if (found) return found
    await page.waitForTimeout(300)
  }
  return null
}

async function findOnce(page, p) {
  if (p.selector) return firstVisible(page.locator(p.selector))
  if (p.name == null) {
    return firstVisible(page.locator('[role=row]:not(:has([role=columnheader])),[role=option],[role=gridcell],[role=treeitem],[data-row],tbody tr,[role=listitem]'))
  }
  const exact = p.name
  for (const role of p.role ? [p.role] : ROLES) {
    const l = await firstVisible(page.getByRole(role, { name: exact, exact: true }))
    if (l) return l
  }
  const loose = new RegExp(esc(exact), 'i')
  for (const role of p.role ? [p.role] : ROLES) {
    const l = await firstVisible(page.getByRole(role, { name: loose }))
    if (l) return l
  }
  return (await firstVisible(page.getByLabel(loose))) ?? (await firstVisible(page.getByTitle(loose))) ?? (await firstVisible(page.getByText(exact, { exact: true }))) ?? (await firstVisible(page.getByText(loose)))
}

/** Runs one plan; resolves to null when done, or to 'missing' / a not-automatable reason. */
async function perform(page, plans) {
  let reason = null
  for (const p of plans) {
    if (p.notAutomatable) { reason ??= p.notAutomatable; continue }
    if (p.key) {
      if (p.focus) await (await findControl(page, p.focus))?.click({ timeout: 1500 }).catch(() => {})
      await page.keyboard.press(p.key).catch(() => {})
      return { done: true, how: `key ${p.key}` }
    }
    const control = await findControl(page, p)
    if (!control) { if (process.env.AUDIT_DEBUG) console.error('not found', p, page.url()); continue }
    const opts = { timeout: 2000 }
    try {
      if (p.verb === 'right-click') await control.click({ ...opts, button: 'right' })
      else if (p.verb === 'double-click') await control.dblclick(opts)
      else if (p.verb === 'ctrl+click') await control.click({ ...opts, modifiers: ['Control'] })
      else if (p.verb === 'shift+click') await control.click({ ...opts, modifiers: ['Shift'] })
      else if (p.verb === 'hover') await control.hover(opts)
      else if (p.verb === 'fill') await control.fill(p.value ?? '')
      else await control.click(opts)
    } catch (e) {
      if (process.env.AUDIT_DEBUG) console.error('click failed', p, String(e).split('\n').slice(0, 3).join(' '))
      continue
    }
    if (p.then) await page.keyboard.press(p.then).catch(() => {})
    if (p.next) {
      await settle(page, 1500)
      const second = await findControl(page, p.next)
      if (!second) return { done: false }
      try { await second.click({ timeout: 2000 }) } catch { return { done: false } }
    }
    return { done: true, how: `${p.verb ?? 'click'} ${p.name ?? p.selector ?? p.generic ?? 'first row'}` }
  }
  if (reason) return { done: false, notAutomatable: reason }
  return { done: false }
}

// ---------- the run ----------
const browser = await chromium.launch()
const fresh = async () => {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } })
  const page = await ctx.newPage()
  page.on('dialog', (d) => d.dismiss().catch(() => {}))
  return { ctx, page }
}

async function pool(items, fn) {
  const queue = [...items]
  const results = []
  await Promise.all(
    Array.from({ length: WORKERS }, async () => {
      const { ctx, page } = await fresh()
      while (queue.length) {
        const item = queue.shift()
        results.push(await fn(page, item))
      }
      await ctx.close()
    }),
  )
  return results
}

// 1. every frame's signature (--reuse-sigs reads the last run's, for a quick re-run while the frames do not change)
const SIGS_FILE = join(OUT, 'frame-signatures.json')
const cached = args['reuse-sigs'] === 'true' ? JSON.parse(readFileSync(SIGS_FILE, 'utf8')) : null
const sigs = cached?.sigs ?? {}
const notBuilt = new Set(cached?.notBuilt ?? [])
if (!cached) await pool(board, async (page, f) => {
  await load(page, frameUrl(f.id))
  const text = await page.innerText('#root').catch(() => '')
  if (/Frame DW-\S+ is not built yet/.test(text)) notBuilt.add(f.id)
  sigs[f.id] = await signature(page)
})

mkdirSync(OUT, { recursive: true })
if (!cached) writeFileSync(SIGS_FILE, JSON.stringify({ sigs, notBuilt: [...notBuilt] }))

// 2. the live roots: every section route, every settings pane, the tray and first-run windows
const ROOT_QUERIES = [
  ...['files', 'shared', 'photos', 'backups', 'trash', 'sync-history', 'available-offline', 'search'].map((s) => `window=main-app&platform=windows&section=${s}`),
  ...['general', 'about', 'account', 'security', 'devices', 'notifications', 'storage', 'backups', 'sync', 'thumbnails', 'compare', 'backup-failures', 'device', 'bandwidth', 'insights', 'recent', 'location', 'diagnostics'].map((p) => `window=main-app&platform=windows&section=settings&sub=${p}`),
  'window=tray&platform=windows',
  'window=onboarding&platform=windows',
]
const live = {}
const liveRoots = []
await pool(ROOT_QUERIES, async (page, q) => {
  await load(page, url(q))
  const state = new Set(await signature(page))
  // A root is every frame whose whole signature is on the live screen (the live window may add its own chrome), as long
  // as it is not a sliver of it: a dialog frame whose few tokens all happen to be there too is not this screen.
  const jac = (sig) => { const s = new Set(sig); const inter = [...s].filter((t) => state.has(t)).length; return inter / (s.size + state.size - inter || 1) }
  const matches = board.map((f) => ({ id: f.id, j: cover(sigs[f.id], state), k: jac(sigs[f.id]) })).filter((m) => m.j >= 0.9 && m.k >= 0.6).sort((x, y) => y.k - x.k)
  liveRoots.push({ query: q, frames: matches.map((m) => `${m.id}:${m.j.toFixed(2)}`) })
  for (const m of matches) if (!live[m.id]) live[m.id] = { url: url(q), steps: [] }
})

// 2b. roots that need a step or two on a section route (DW-0401 is Files with a row selected), checked before use
for (const [id, hint] of Object.entries(overrides._roots ?? {})) {
  if (live[id]) continue
  const { ctx, page } = await fresh()
  await load(page, url(hint.query))
  let done = true
  for (const step of hint.steps) { const r = await perform(page, step); if (!r.done) { done = false; break } await page.waitForTimeout(SETTLE_MS) }
  const state = new Set(await signature(page))
  const c = cover(sigs[id], state)
  // `declared`: the live screen shows the frame's state with the live source's own data (the frame renders the board's
  // example rows), so the signatures differ by data, not by screen; the hint says why.
  const used = done && (c >= 0.75 || Boolean(hint.declared))
  liveRoots.push({ query: hint.query, steps: hint.steps.length, frames: [`${id}:${c.toFixed(2)}`], used, declared: hint.declared })
  if (used) live[id] = { url: url(hint.query), steps: hint.steps }
  await ctx.close()
}

// 3. BFS levels over the board from the live roots
const level = {}
{
  let frontier = Object.keys(live)
  frontier.forEach((id) => (level[id] = 0))
  let d = 0
  while (frontier.length) {
    const next = []
    for (const id of frontier) for (const a of actions.filter((x) => x.source === id)) for (const t of a.to) if (level[t] == null) { level[t] = d + 1; next.push(t) }
    frontier = next
    d++
  }
  for (const f of board) level[f.id] ??= 99
}
const byLevel = {}
for (const a of actions) if (!ONLY || ONLY.has(a.source)) (byLevel[level[a.source]] ??= []).push(a)

const unavailable = (page) => {
  const seen = []
  const on = (m) => {
    const t = m.text()
    const hit = t.match(/^(\S+) unavailable$/)
    if (hit) seen.push(hit[1])
  }
  page.on('console', on)
  return { seen, stop: () => page.off('console', on) }
}


async function openSource(page, id) {
  const entry = live[id]
  if (entry) {
    await load(page, entry.url)
    let ok = true
    for (const step of entry.steps) {
      const r = await perform(page, step)
      if (!r.done) { ok = false; break }
      await settle(page, 2500)
      if (step.follow) { const u = new URL(page.url()); u.searchParams.delete('frame'); await load(page, u.toString()) }
    }
    if (ok) return { mode: 'live', steps: entry.steps.length }
  }
  await load(page, frameUrl(id))
  return { mode: 'frame', steps: 0 }
}

async function audit(page, a) {
  const row = { key: a.key, source: a.source, batch: batchOf[a.source] ?? null, on: a.on, does: a.does, to: a.to, planted: a.planted || undefined }
  const plans = PLANT && a.planted ? planFor({ ...a, key: 'planted' }) : planFor(a)
  if (plans.every((p) => p.notAutomatable)) return { ...row, outcome: 'not-automatable', detail: plans[0].notAutomatable }
  if (notBuilt.has(a.source)) return { ...row, outcome: 'missing-control', detail: 'the source frame is not built' }
  const entry = await openSource(page, a.source)
  row.entry = entry.mode === 'live' ? `live(${entry.steps} steps)` : 'frame'
  const preUrl = page.url()
  const pre = new Set(await signature(page))
  // Commands the window calls on its own (sync_status every second, the theme's desktop_config) are not the control's.
  const idle = unavailable(page)
  await page.waitForTimeout(SETTLE_MS)
  idle.stop()
  const background = new Set(idle.seen)
  const log = unavailable(page)
  const r = await perform(page, plans)
  await settle(page, 2500)
  log.stop()
  if (!r.done) return r.notAutomatable ? { ...row, outcome: 'not-automatable', detail: r.notAutomatable } : { ...row, outcome: 'missing-control', detail: `no control for "${a.on}"` }
  row.how = r.how
  let followed = false
  const postUrl = new URL(page.url())
  if (postUrl.toString() !== preUrl && entry.mode === 'frame' && postUrl.searchParams.get('frame')) {
    // A section route changed under a registry renderer: follow it into the live window.
    postUrl.searchParams.delete('frame')
    await load(page, postUrl.toString())
    followed = true
  }
  const post = new Set(await signature(page))
  const changed = followed || page.url() !== preUrl || !same(pre, post)
  const cmds = [...new Set(log.seen)].filter((c) => !background.has(c) && !BACKGROUND_COMMANDS.has(c))
  if (cmds.length) row.commands = cmds
  const ownerOf = (c) => Object.entries(owners).find(([, o]) => o.commands.some((x) => (x.endsWith('*') ? c.startsWith(x.slice(0, -1)) : x === c)))?.[0] ?? null

  if (!a.to.length) {
    if (changed) return { ...row, outcome: 'ok', detail: 'no target frame; the screen changed' }
    if (cmds.length) return { ...row, outcome: 'command-unsupported', detail: cmds.join(', '), owner: ownerOf(cmds[0]) }
    return { ...row, outcome: 'no-effect' }
  }
  // Which frame does the screen show now? Each frame scores on its whole signature and on what it adds to the screen the
  // action started from; a sibling variant of the target (DW-0408 / DW-0408b: the same screen, other data) counts.
  const family = (id) => id.replace(/[a-z]$/, '')
  const score = (id) => {
    const sig = sigs[id]
    const distinct = sig.filter((x) => !pre.has(x))
    // A frame that adds nothing to the starting screen is the starting screen: a weak answer once the screen changed.
    return 0.5 * cover(sig, post) + 0.5 * (distinct.length ? cover(distinct, post) : changed ? 0.3 : 1)
  }
  const ranked = board.map((f) => ({ id: f.id, s: score(f.id) })).sort((x, y) => y.s - x.s)
  // `_same` (controls.json): frames that are one screen with other data or selection, e.g. the Files view.
  const sameScreen = (t) => (overrides._same ?? []).find((g) => g.frames.includes(t))?.frames ?? [t]
  const targets = new Set(a.to.flatMap(sameScreen).flatMap((t) => board.filter((f) => family(f.id) === family(t)).map((f) => f.id)))
  const bestTarget = ranked.find((r) => targets.has(r.id))
  const bestOther = ranked.find((r) => !targets.has(r.id))
  if (bestTarget && bestTarget.s >= 0.6 && bestTarget.s >= bestOther.s - 0.05 && (changed || bestTarget.s >= 0.95)) {
    const t = bestTarget.id
    if (entry.mode === 'live' && !live[t]) live[t] = { url: live[a.source].url, steps: [...live[a.source].steps, followed ? plans.map((p) => ({ ...p, follow: true })) : plans] }
    return { ...row, outcome: 'ok', reached: t, score: Number(bestTarget.s.toFixed(2)) }
  }
  row.score = bestTarget ? Number(bestTarget.s.toFixed(2)) : 0
  if (cmds.length) return { ...row, outcome: 'command-unsupported', detail: cmds.join(', '), owner: ownerOf(cmds[0]) }
  const elsewhere = a.to.filter((t) => windowOf(t) !== windowOf(a.source))
  if (elsewhere.length === a.to.length) return { ...row, outcome: 'not-automatable', detail: `the target is another window (${elsewhere.map((t) => `${t}: ${windowOf(t)}`).join(', ')}); Tauri opens it, a browser cannot` }
  if (!changed) return { ...row, outcome: 'no-effect' }
  if (bestOther.s >= 0.6) return { ...row, outcome: 'wrong-target', reached: bestOther.id, detail: `reached ${bestOther.id} (score ${bestOther.s.toFixed(2)}; target ${row.score})` }
  return { ...row, outcome: 'wrong-target', reached: null, detail: `an unrecognised screen (target score ${row.score}): ${[...post].filter((x) => !pre.has(x)).slice(0, 6).join(' | ')}` }
}

const rows = []
for (const lvl of Object.keys(byLevel).map(Number).sort((x, y) => x - y)) {
  // Frames on one level run together; a frame's actions run in order so a level's ok rows can open the next level live.
  rows.push(...(await pool(byLevel[lvl], audit)))
}
await browser.close()
rows.sort((x, y) => actions.findIndex((a) => a.key === x.key) - actions.findIndex((a) => a.key === y.key))

// ---------- the report ----------
const OUTCOMES = ['ok', 'missing-control', 'no-effect', 'wrong-target', 'command-unsupported', 'not-automatable']
const totals = Object.fromEntries(OUTCOMES.map((o) => [o, rows.filter((r) => r.outcome === o).length]))
const perBatch = {}
for (const r of rows) {
  const b = r.batch ?? 'none'
  perBatch[b] ??= Object.fromEntries(OUTCOMES.map((o) => [o, 0]))
  perBatch[b][r.outcome]++
}
mkdirSync(OUT, { recursive: true })
const name = PLANT ? 'map-audit-planted' : args.name ?? 'map-audit'
writeFileSync(join(OUT, `${name}.json`), JSON.stringify({ base: BASE, actions: rows.length, totals, perBatch, liveRoots, notBuilt: [...notBuilt], rows }, null, 1))
const md = [
  `# Desktop v2 action map audit${PLANT ? ' (planted label)' : ''}`,
  '',
  `${rows.length} actions from ${board.length} frames (Mail Backup excluded). Generated by \`scripts/v2-map-audit/audit.mjs\`.`,
  `Rows opened live: ${rows.filter((r) => r.entry?.startsWith('live')).length}; through the frame renderer: ${rows.filter((r) => r.entry === 'frame').length}; not opened (not automatable or unbuilt): ${rows.filter((r) => !r.entry).length}.`,
  '',
  '| Outcome | Count |',
  '|---|---|',
  ...OUTCOMES.map((o) => `| ${o} | ${totals[o]} |`),
  '',
  '## Per batch',
  '',
  `| Batch | ${OUTCOMES.join(' | ')} |`,
  `|---|${OUTCOMES.map(() => '---').join('|')}|`,
  ...Object.keys(perBatch).sort().map((b) => `| ${b} | ${OUTCOMES.map((o) => perBatch[b][o]).join(' | ')} |`),
  '',
  '## Rows that are not ok',
  '',
  '| Action | Batch | On | To | Outcome | Detail |',
  '|---|---|---|---|---|---|',
  ...rows.filter((r) => r.outcome !== 'ok').map((r) => `| ${r.key} | ${r.batch ?? ''} | ${r.on.replace(/\|/g, '/')} | ${r.to.join(' ')} | ${r.outcome}${r.owner ? ` (${r.owner})` : ''} | ${(r.detail ?? '').replace(/\|/g, '/')} |`),
  '',
]
writeFileSync(join(OUT, `${name}.md`), md.join('\n'))
console.log(`actions=${rows.length} ${OUTCOMES.map((o) => `${o}=${totals[o]}`).join(' ')}`)
if (PLANT) {
  const p = rows.find((r) => r.planted)
  console.log(`planted ${p?.key}: ${p?.outcome}`)
  process.exit(p?.outcome === 'missing-control' ? 0 : 1)
}
