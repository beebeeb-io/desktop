/**
 * What the popover paints for every state (task 1683 slice 3; spec sections 3 and 10).
 *
 * Static markup of the presentational `PopoverSurface` for every state, against the same snapshots
 * the model tests and the Playwright rung use. This is structure only: layout, geometry, focus and
 * contrast are measured in a real browser (`tests/render-mac-popover.mjs`). What it pins here is
 * what must hold before any browser sees it: every control is a real <button>, every progress bar
 * has a name, every state has one title and one dialog, the status row is a status, and no colour
 * is a literal.
 *
 * Mutation checks (red first, then reverted) are recorded in the task Notes, 2026-10-01.
 */
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { parsePopoverSnapshot } from '../src/popoverContract'
import { PopoverSurface, type SurfaceHandlers } from '../src/macPopover/MacPopover'
import { DEFAULT_CONTEXT, loadFailView, loadingView, viewForSnapshot, type PopoverView, type ViewContext } from '../src/macPopover/model'
// @ts-expect-error - a plain .mjs shared with the Playwright rung
import { NOW, SNAPSHOTS } from './fixtures/macPopoverStates.mjs'

const noop = () => {}
const handlers: SurfaceHandlers = { onAction: noop, onGear: noop, onUnlock: noop, onReview: noop, onOpenFinder: noop, onViewOnline: noop, onAddStorage: noop }
const ctx = (patch: Partial<ViewContext> = {}): ViewContext => ({ ...DEFAULT_CONTEXT, now: NOW, locale: 'nl-NL', ...patch })
const snapshot = (key: string) => parsePopoverSnapshot(JSON.parse(JSON.stringify(SNAPSHOTS[key])))!

const STATES: Record<string, PopoverView> = {
  loading: loadingView(),
  loadfail: loadFailView(),
  ...Object.fromEntries(Object.keys(SNAPSHOTS).map((k) => [k, viewForSnapshot(snapshot(k), ctx())])),
  unlock: viewForSnapshot(snapshot('locked'), ctx({ unlockOpen: true, unlockWrong: true })),
  finderadding: viewForSnapshot(snapshot('finder'), ctx({ finderPending: true })),
}
const html = (v: PopoverView, notice: { text: string; detail: string | null } | null = null) => renderToStaticMarkup(<PopoverSurface view={v} notice={notice} busy={false} handlers={handlers} />)
const count = (s: string, pattern: RegExp) => (s.match(pattern) ?? []).length
const NAMES = Object.keys(STATES)

describe('every state paints one dialog with real controls', () => {
  test('the 20 states render (instrument: the list below has the states the spec names)', () => {
    expect(NAMES.length).toBe(19)
    for (const id of ['signedout', 'ended', 'locked', 'unlock', 'paused', 'error', 'offline', 'storage', 'finder', 'finderadding', 'finderfail', 'finderoff', 'synced', 'conflict', 'empty', 'syncing', 'syncing0', 'loading', 'loadfail']) {
      expect(NAMES.includes(id), id).toBe(true)
      expect(STATES[id].state).toBe(id)
    }
  })

  test('one dialog named Beebeeb per state, carrying its state id', () => {
    for (const [id, v] of Object.entries(STATES)) {
      const out = html(v)
      expect(count(out, /role="dialog"/g), id).toBe(1)
      expect(out.includes('aria-label="Beebeeb"'), id).toBe(true)
      expect(out.includes(`data-state="${id}"`), id).toBe(true)
    }
  })

  test('every control is a <button> or an <input>: no div with a button role, no link-like span', () => {
    for (const [id, v] of Object.entries(STATES)) {
      const out = html(v)
      expect(out.includes('role="button"'), id).toBe(false)
      expect(out.includes('onclick'), id).toBe(false)
    }
  })

  test('buttons per state, counted: gear, the state’s actions, and the footer', () => {
    const expected: Record<string, number> = {
      loading: 1, loadfail: 2, synced: 4, conflict: 5, empty: 4, syncing: 4, syncing0: 4, locked: 5, unlock: 5, paused: 5, error: 5, offline: 4, storage: 5,
      signedout: 2, ended: 2, finder: 5, finderadding: 5, finderfail: 5, finderoff: 5,
    }
    const got = Object.fromEntries(NAMES.map((id) => [id, count(html(STATES[id]), /<button\b/g)]))
    expect(got).toEqual(expected)
  })

  test('the gear is a menu button in every state, with a name', () => {
    for (const [id, v] of Object.entries(STATES)) {
      const out = html(v)
      expect(count(out, /aria-label="Beebeeb menu"/g), id).toBe(1)
      expect(count(out, /aria-haspopup="menu"/g), id).toBe(1)
    }
  })

  test('the footer is the same three actions in every state that has one, and absent for signed out, session ended and the two blank frames', () => {
    const withFooter = NAMES.filter((id) => html(STATES[id]).includes('class="mp-footer"'))
    expect(withFooter.length).toBe(15)
    expect(NAMES.filter((id) => !withFooter.includes(id)).sort()).toEqual(['ended', 'loadfail', 'loading', 'signedout'])
    for (const id of withFooter) {
      const out = html(STATES[id])
      expect([...out.matchAll(/<span>?[^<]*<\/span>|>(Open in Finder|View online|Add storage)</g)].length, id).toBeGreaterThan(0)
      for (const label of ['Open in Finder', 'View online', 'Add storage']) expect(out.includes(label), `${id} ${label}`).toBe(true)
    }
  })

  test('Open in Finder is the one disabled footer action, and only in the four Finder states', () => {
    const disabledFooter = (id: string) => /<div class="mp-footer"[^>]*><button[^>]*disabled=""/.test(html(STATES[id]))
    expect(NAMES.filter(disabledFooter).sort()).toEqual(['finder', 'finderadding', 'finderfail', 'finderoff'])
  })
})

describe('headings, status and names', () => {
  test('a message state has exactly one title (h1) and the fixed 56 pt top inset that puts it at the same y', () => {
    const messageStates = NAMES.filter((id) => count(html(STATES[id]), /<h1\b/g) === 1)
    expect(messageStates.sort()).toEqual(['empty', 'ended', 'error', 'finder', 'finderadding', 'finderfail', 'finderoff', 'loadfail', 'locked', 'offline', 'paused', 'signedout', 'storage', 'unlock'])
    for (const id of messageStates) {
      const out = html(STATES[id])
      expect(out.includes('padding:56px 32px 0'), `${id}: the body's top inset`).toBe(true)
      expect(count(out, /data-mp-title/g), id).toBe(1)
    }
    expect(messageStates.length).toBe(14)
  })

  test('a list state has no h1 and has section headings; the blank first frame has neither', () => {
    for (const id of ['synced', 'conflict', 'syncing', 'syncing0']) {
      const out = html(STATES[id])
      expect(count(out, /<h1\b/g), id).toBe(0)
      expect(count(out, /<h2\b/g), id).toBeGreaterThan(0)
    }
    expect(count(html(STATES.loading), /<h[12]\b/g)).toBe(0)
  })

  test('role=status exists only on the status row, only in the five states that have one', () => {
    const withStatus = NAMES.filter((id) => html(STATES[id]).includes('role="status"'))
    expect(withStatus.sort()).toEqual(['conflict', 'empty', 'synced', 'syncing', 'syncing0'])
    expect(html(STATES.synced).includes('title="Last checked 2 min ago"')).toBe(true)
  })

  test('every progress bar has a name; the storage bar’s value is the percentage the model computed', () => {
    let bars = 0
    for (const [id, v] of Object.entries(STATES)) {
      const out = html(v)
      for (const tag of out.match(/<div[^>]*role="progressbar"[^>]*>/g) ?? []) {
        bars += 1
        expect(/aria-label="[^"]+"/.test(tag), `${id}: ${tag}`).toBe(true)
        expect(/aria-valuemin="0"/.test(tag) && /aria-valuemax="100"/.test(tag), `${id}: ${tag}`).toBe(true)
      }
    }
    // 13 states with a storage figure + the overall bar in b and b prime + 3 per-file bars in b
    expect(bars).toBe(13 + 2 + 3)
    expect(html(STATES.synced).includes('aria-label="Storage used" aria-valuemin="0" aria-valuemax="100" aria-valuenow="42"')).toBe(true)
    // the overall bar of b prime is indeterminate: it has a name and NO value (never a made-up one)
    const b0 = html(STATES.syncing0).match(/<div[^>]*aria-label="Overall progress"[^>]*>/)![0]
    expect(b0.includes('aria-valuenow')).toBe(false)
    expect(b0.includes('mp-indeterminate')).toBe(true)
    const b = html(STATES.syncing).match(/<div[^>]*aria-label="Overall progress"[^>]*>/)![0]
    expect(b.includes('aria-valuenow="38"')).toBe(true)
  })

  test('b: three per-file bars with the file’s name; b prime: none', () => {
    const named = (out: string) => (out.match(/role="progressbar"/g) ?? []).length
    expect(named(html(STATES.syncing))).toBe(1 + 1 + 3) // storage, overall, 3 files
    expect(named(html(STATES.syncing0))).toBe(1 + 1)
    expect(html(STATES.syncing).includes('aria-label="Interview 09, raw audio.wav"')).toBe(true)
  })

  test('the unlock form is a real password field with a name, an error line that is an alert, and aria-invalid on failure', () => {
    const out = html(STATES.unlock)
    expect(out.includes('type="password"')).toBe(true)
    expect(out.includes('aria-label="Vault password"')).toBe(true)
    expect(out.includes('aria-invalid="true"')).toBe(true)
    expect(out.includes('autoComplete="current-password"')).toBe(true)
    expect(/id="mp-unlock-line" role="alert"/.test(out)).toBe(true)
    expect(out.includes('That password didn’t work. Try again.')).toBe(true)
    const calm = html(viewForSnapshot(snapshot('locked'), ctx({ unlockOpen: true })))
    expect(calm.includes('aria-invalid')).toBe(false)
    expect(calm.includes('That password')).toBe(false)
  })

  test('a row is one named group; the arrow and the tile are decorative', () => {
    const out = html(STATES.synced)
    expect(count(out, /role="group" aria-label="[^"]+"/g)).toBe(5)
    expect(out.includes('aria-label="Q3 ledger reconciliation.xlsx, Investigations, 2 minutes ago, uploaded"')).toBe(true)
  })

  test('the notice strip is one alert and is drawn only when there is a notice', () => {
    expect(html(STATES.paused).includes('Couldn’t resume sync.')).toBe(false)
    const out = html(STATES.paused, { text: 'Couldn’t resume sync.', detail: 'engine busy' })
    expect(count(out, /role="alert"/g)).toBe(1)
    expect(out.includes('Couldn’t resume sync.')).toBe(true)
    expect(out.includes('engine busy')).toBe(true)
  })
})

describe('brand rules in the markup', () => {
  test('no colour literal anywhere in any state (hex, rgb, hsl, oklch, named white or black): 19 states', () => {
    for (const [id, v] of Object.entries(STATES)) {
      const out = html(v)
      expect(/#[0-9a-fA-F]{3,8}\b/.test(out.replace(/&#\d+;/g, '')), `${id}: hex`).toBe(false)
      expect(/\b(rgb|rgba|hsl|hsla|oklch)\(/.test(out), `${id}: color function`).toBe(false)
      expect(/(?:color|background|border|fill|stroke):\s*(?:white|black)\b/.test(out), `${id}: named colour`).toBe(false)
    }
    expect(NAMES.length).toBe(19)
  })

  test('no emoji in any state’s text', () => {
    for (const [id, v] of Object.entries(STATES)) expect(/\p{Extended_Pictographic}/u.test(html(v).replace(/<[^>]+>/g, '')), id).toBe(false)
  })

  test('amber appears only for encryption state, bytes being encrypted, and the primary action (counted per state)', () => {
    // `var(--amber…)` occurrences in INLINE styles. The primary button's amber is a class (`mp-primary`), counted separately.
    const amber = (id: string) => count(html(STATES[id]), /var\(--amber/g)
    const got = Object.fromEntries(NAMES.map((id) => [id, amber(id)]))
    expect(got).toEqual({
      loading: 0, loadfail: 0,
      synced: 1, conflict: 1,                      // the Encrypted lock on the status row, nothing else
      empty: 1 + 4,                                // + the hex lock on the folder art (fill, stroke, 2 x ink)
      syncing: 1 + 2, syncing0: 1,                 // + the 2 upload bars (bytes being encrypted); downloads are neutral
      locked: 2, unlock: 2,                        // the lock art: amber-bg and amber-deep
      paused: 0, error: 0, offline: 0, storage: 0,
      signedout: 0, ended: 0, finder: 0, finderadding: 0, finderfail: 0, finderoff: 0,
    })
    // never on the avatar, the storage bar, the file tiles, the header or the footer
    for (const id of NAMES) {
      const out = html(STATES[id])
      const header = out.slice(0, out.indexOf('flex:1 1 0%') > 0 ? out.indexOf('flex:1 1 0%') : 400)
      expect(header.includes('amber'), `${id}: header`).toBe(false)
    }
    const footer = (id: string) => html(STATES[id]).split('class="mp-footer"')[1] ?? ''
    for (const id of NAMES) expect(footer(id).includes('amber'), `${id}: footer`).toBe(false)
  })

  test('the primary action is the only amber button: one per message state that has an action, none in list states', () => {
    const primaries = Object.fromEntries(NAMES.map((id) => [id, count(html(STATES[id]), /class="mp-primary"/g)]))
    expect(primaries).toEqual({
      loading: 0, loadfail: 1, synced: 0, conflict: 0, empty: 0, syncing: 0, syncing0: 0, locked: 1, unlock: 1, paused: 1, error: 1, offline: 0, storage: 1,
      signedout: 1, ended: 1, finder: 1, finderadding: 1, finderfail: 1, finderoff: 1,
    })
  })
})
