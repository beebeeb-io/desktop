/**
 * The macOS Beebeeb window's pure decisions (task 1683 slice 1; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6, §7.8, §8.2, §9.3, §11): which view a navigation request
 * lands on, what the Settings back control says, where focus goes, what the menu may offer, and what Home's status
 * row says. Nothing here renders; tests/macApp.test.tsx runs the window itself.
 */
import { afterEach, describe, expect, jest, test } from 'bun:test'
import {
  backLabel,
  focusAfterNav,
  heldNoteGoesToSettings,
  homeStatus,
  initialMainView,
  lastCheckedLine,
  listAfterKey,
  MAIN_SECTIONS,
  mainButtonId,
  menuStateFor,
  navigate,
  parseMainNav,
  sectionTitle,
  settingsSections,
  coerceSettingsView,
  type MainView,
  type NavContext,
} from '../src/mainWindowModel'

const home: MainView = { mode: 'main', section: 'home' }
const review: MainView = { mode: 'main', section: 'review' }
const trash: MainView = { mode: 'main', section: 'trash' }
const signedIn: NavContext = { signedIn: true, lastSettings: null, sheetOpen: false }
const signedOut: NavContext = { signedIn: false, lastSettings: null, sheetOpen: false }

describe('the two sidebar lists', () => {
  test('signed in: Home, Review and Trash in the window sidebar (slice 4); the Settings list is Account, General, Sync, Security, About (slice 5 adds Security)', () => {
    expect(MAIN_SECTIONS.map((s) => s.label)).toEqual(['Home', 'Review', 'Trash'])
    expect(MAIN_SECTIONS.map((s) => s.id)).toEqual(['home', 'review', 'trash'])
    expect(settingsSections(true)).toEqual(['account', 'general', 'sync', 'security', 'about'])
  })

  test('signed out: Settings shows General and About only (§7.8)', () => {
    expect(settingsSections(false)).toEqual(['general', 'about'])
  })
})

describe('a section that exists only with a session is never left up without one (§7.8, review P2-6 and its delta review)', () => {
  const security = { mode: 'settings', section: 'security', returnTo: 'home' } as const

  test('signed out, Security falls back to General: its sessions, devices and activity belong to an account that is gone', () => {
    expect(coerceSettingsView(security, false)).toEqual({ mode: 'settings', section: 'general', returnTo: 'home' })
  })

  test('so does Sync: signed out only General and About exist (§7.8)', () => {
    expect(coerceSettingsView({ ...security, section: 'sync' }, false)).toEqual({ mode: 'settings', section: 'general', returnTo: 'home' })
  })

  test('Account stays while the sign-out finishes: it is where "You’re signed out" and the held sentence are shown until the window remounts on the sign-in card', () => {
    const account = { ...security, section: 'account' } as const
    expect(coerceSettingsView(account, false)).toBe(account)
  })

  test('the two sections a signed-out window has stay as they are', () => {
    for (const section of ['general', 'about'] as const) {
      const view = { ...security, section }
      expect(coerceSettingsView(view, false)).toBe(view)
    }
  })

  test('signed in, and anywhere but Settings, the view is returned as it is (the same object, so nothing re-renders)', () => {
    expect(coerceSettingsView(security, true)).toBe(security)
    const sync = { ...security, section: 'sync' } as const
    expect(coerceSettingsView(sync, true)).toBe(sync)
    const home = { mode: 'main', section: 'home' } as const
    expect(coerceSettingsView(home, false)).toBe(home)
  })
})

describe('navigate (§7.8, §9.3)', () => {
  test('⌘, with no section opens General the first time, then the last section used in this window', () => {
    expect(navigate(home, { to: 'settings', section: null }, signedIn)).toEqual({ mode: 'settings', section: 'general', returnTo: 'home' })
    expect(navigate(home, { to: 'settings', section: null }, { ...signedIn, lastSettings: 'sync' })).toEqual({ mode: 'settings', section: 'sync', returnTo: 'home' })
  })

  test('an opener that names a section opens that one', () => {
    expect(navigate(home, { to: 'settings', section: 'about' }, { ...signedIn, lastSettings: 'sync' })).toEqual({ mode: 'settings', section: 'about', returnTo: 'home' })
  })

  test('signed out, a section that needs a session is never shown: the last section and a named one fall back to General', () => {
    expect(navigate(home, { to: 'settings', section: 'account' }, signedOut)).toEqual({ mode: 'settings', section: 'general', returnTo: 'home' })
    expect(navigate(home, { to: 'settings', section: null }, { ...signedOut, lastSettings: 'sync' })).toEqual({ mode: 'settings', section: 'general', returnTo: 'home' })
    expect(navigate(home, { to: 'settings', section: 'about' }, signedOut)).toEqual({ mode: 'settings', section: 'about', returnTo: 'home' })
  })

  test('back and ⌘[ return to where Settings was entered from; ⌘1 leaves Settings for Home', () => {
    const settings: MainView = { mode: 'settings', section: 'sync', returnTo: 'home' }
    expect(navigate(settings, { to: 'back' }, signedIn)).toEqual(home)
    expect(navigate(settings, { to: 'home' }, signedIn)).toEqual(home)
  })

  test('back outside Settings changes nothing', () => {
    expect(navigate(home, { to: 'back' }, signedIn)).toBe(home)
    expect(navigate(review, { to: 'back' }, signedIn)).toBe(review)
  })

  test('⌘2 and ⌘3 land on Review and Trash, from the main sections and from Settings (§9.3)', () => {
    expect(navigate(home, { to: 'review' }, signedIn)).toEqual(review)
    expect(navigate(home, { to: 'trash' }, signedIn)).toEqual(trash)
    expect(navigate(trash, { to: 'review' }, signedIn)).toEqual(review)
    const settings: MainView = { mode: 'settings', section: 'sync', returnTo: 'home' }
    expect(navigate(settings, { to: 'review' }, signedIn)).toEqual(review)
    expect(navigate(settings, { to: 'trash' }, signedIn)).toEqual(trash)
  })

  test('asking for the section already showing changes nothing (the same view back, so focus does not jump)', () => {
    // The views a window holds come from `navigate` and `initialMainView`; asking again for the one it holds is the
    // same object, which is how `MacApp` tells "nothing changed" from "moved".
    const showingReview = navigate(home, { to: 'review' }, signedIn)
    const showingTrash = navigate(home, { to: 'trash' }, signedIn)
    expect(navigate(showingReview, { to: 'review' }, signedIn)).toBe(showingReview)
    expect(navigate(showingTrash, { to: 'trash' }, signedIn)).toBe(showingTrash)
    expect(navigate(initialMainView(''), { to: 'home' }, signedIn)).toBe(initialMainView(''))
    expect(navigate(navigate(showingReview, { to: 'settings', section: null }, signedIn), { to: 'back' }, signedIn)).toBe(showingReview)
  })

  test('Settings remembers where it was entered from, and back returns to Review or Trash, not always Home', () => {
    expect(navigate(review, { to: 'settings', section: null }, signedIn)).toEqual({ mode: 'settings', section: 'general', returnTo: 'review' })
    const fromTrash = navigate(trash, { to: 'settings', section: 'about' }, signedIn)
    expect(fromTrash).toEqual({ mode: 'settings', section: 'about', returnTo: 'trash' })
    expect(navigate(fromTrash, { to: 'back' }, signedIn)).toEqual(trash)
  })

  test('§8.2: with a sheet open ⌘2 and ⌘3 change nothing either (a conflict sheet over Review, a password sheet over Trash)', () => {
    const sheet = { ...signedIn, sheetOpen: true }
    expect(navigate(review, { to: 'trash' }, sheet)).toBe(review)
    expect(navigate(trash, { to: 'review' }, sheet)).toBe(trash)
    expect(navigate(trash, { to: 'home' }, sheet)).toBe(trash)
  })

  test('switching sections inside Settings keeps the place it returns to', () => {
    const settings: MainView = { mode: 'settings', section: 'general', returnTo: 'home' }
    expect(navigate(settings, { to: 'settings', section: 'about' }, signedIn)).toEqual({ mode: 'settings', section: 'about', returnTo: 'home' })
  })

  test('§8.2: while a sheet is open no request moves the window, so ⌘1, ⌘[ and ⌘, cannot unmount Account under its sheet', () => {
    const account: MainView = { mode: 'settings', section: 'account', returnTo: 'home' }
    const sheet = { ...signedIn, sheetOpen: true }
    for (const nav of [{ to: 'home' }, { to: 'back' }, { to: 'settings', section: null }, { to: 'settings', section: 'general' }] as const) {
      expect(navigate(account, nav, sheet)).toBe(account)
    }
  })
})

describe('parseMainNav: what Rust sends on main:navigate', () => {
  test('the request kinds, and a section only when it is a real one', () => {
    expect(parseMainNav({ to: 'home' })).toEqual({ to: 'home' })
    expect(parseMainNav({ to: 'review' })).toEqual({ to: 'review' })
    expect(parseMainNav({ to: 'trash' })).toEqual({ to: 'trash' })
    expect(parseMainNav({ to: 'back' })).toEqual({ to: 'back' })
    expect(parseMainNav({ to: 'settings', section: 'about' })).toEqual({ to: 'settings', section: 'about' })
    expect(parseMainNav({ to: 'settings', section: null })).toEqual({ to: 'settings', section: null })
    expect(parseMainNav({ to: 'settings' })).toEqual({ to: 'settings', section: null })
  })

  // Review P3-1: `open_reauth_window` asks for “Sign in again” whatever the session says (Linux's `mode=reauth`).
  test('“Sign in again” is a request of its own, with nothing else in it', () => {
    expect(parseMainNav({ to: 'reauth' })).toEqual({ to: 'reauth' })
  })

  test('missing keys can request Set up this Mac in the existing window', () => {
    expect(parseMainNav({ to: 'setup' })).toEqual({ to: 'setup' })
  })

  test('anything else is no request', () => {
    for (const raw of [null, undefined, 'home', {}, { to: 'files' }, { to: 'settings', section: 'status' }, { to: 'settings', section: 7 }]) {
      expect(parseMainNav(raw)).toBeNull()
    }
  })

  test('Security is a section a request can name (slice 5; it was the example of an unknown one in slice 1)', () => {
    expect(parseMainNav({ to: 'settings', section: 'security' })).toEqual({ to: 'settings', section: 'security' })
  })
})

describe('navigate: “Sign in again” (review P3-1)', () => {
  test('it leaves Settings for Home, where the card is shown, and moves nothing under a sheet', () => {
    const inSettings: MainView = { mode: 'settings', section: 'account', returnTo: 'home' }
    expect(navigate(inSettings, { to: 'reauth' }, signedIn)).toEqual(home)
    expect(navigate(inSettings, { to: 'reauth' }, { ...signedIn, sheetOpen: true })).toBe(inSettings)
  })
})

describe('where a held sign-out sentence is shown (accountSession.ts; review P2-1)', () => {
  // A sentence that becomes held while the window is mounted brings a signed-in window to Settings › Account, where
  // the sign-out note lives. A window that is showing a sign-in card shows the sentence above that card instead
  // (`SignInWindow`'s note), so it is not taken away from the card: the account switch holds its own sentence while
  // its card is up, and the remount that follows draws the signed-out card with it.
  test('only a signed-in window that is not showing a card goes to Settings', () => {
    expect(heldNoteGoesToSettings({ signedIn: true, showingCard: false })).toBe(true)
    expect(heldNoteGoesToSettings({ signedIn: true, showingCard: true })).toBe(false)
    expect(heldNoteGoesToSettings({ signedIn: false, showingCard: false })).toBe(false)
    expect(heldNoteGoesToSettings({ signedIn: false, showingCard: true })).toBe(false)
  })
})

describe('the back control (§7.8, §10.2 #48)', () => {
  test('names where it returns to, or Sign in while signed out', () => {
    expect(backLabel('home', true)).toBe('Home')
    expect(backLabel('review', true)).toBe('Review')
    expect(backLabel('trash', true)).toBe('Trash')
    expect(backLabel('review', false)).toBe('Sign in')
    expect(backLabel('home', false)).toBe('Sign in')
  })
})

describe('the toolbar title is the section name (§7.8)', () => {
  test('Home, and each Settings section by its label', () => {
    expect(sectionTitle(home)).toBe('Home')
    expect(sectionTitle(review)).toBe('Review')
    expect(sectionTitle(trash)).toBe('Trash')
    expect(sectionTitle({ mode: 'settings', section: 'account', returnTo: 'home' })).toBe('Account')
    expect(sectionTitle({ mode: 'settings', section: 'about', returnTo: 'home' })).toBe('About')
  })
})

describe('focus (§11)', () => {
  test('entering Settings focuses the selected section; leaving it by back focuses the gear; ⌘1 focuses Home', () => {
    const settings: MainView = { mode: 'settings', section: 'sync', returnTo: 'home' }
    expect(focusAfterNav(home, settings, { to: 'settings', section: null })).toBe('mw-settings-sync')
    expect(focusAfterNav(settings, home, { to: 'back' })).toBe('mw-gear')
    expect(focusAfterNav(settings, home, { to: 'home' })).toBe('mw-section-home')
  })

  test('⌘2 and ⌘3 put focus on the section they chose; back from a Settings entered on Review still focuses the gear', () => {
    expect(focusAfterNav(home, review, { to: 'review' })).toBe('mw-section-review')
    expect(focusAfterNav(review, trash, { to: 'trash' })).toBe('mw-section-trash')
    const settings: MainView = { mode: 'settings', section: 'sync', returnTo: 'review' }
    expect(focusAfterNav(settings, review, { to: 'back' })).toBe('mw-gear')
    expect(mainButtonId('review')).toBe('mw-section-review')
  })

  test('a request that changed nothing moves no focus', () => {
    expect(focusAfterNav(home, home, { to: 'back' })).toBeNull()
  })
})

describe('the keys of either sidebar list (§7.6, §11: ↑/↓ move and select, Home and End jump)', () => {
  const list = ['account', 'general', 'sync', 'security', 'about'] as const
  test('down and up move and wrap, Home and End jump', () => {
    expect(listAfterKey(list, 'general', 'ArrowDown')).toBe('sync')
    expect(listAfterKey(list, 'about', 'ArrowDown')).toBe('account')
    expect(listAfterKey(list, 'account', 'ArrowUp')).toBe('about')
    expect(listAfterKey(list, 'sync', 'Home')).toBe('account')
    expect(listAfterKey(list, 'sync', 'End')).toBe('about')
  })

  test('left, right and every other key are left alone', () => {
    for (const key of ['ArrowLeft', 'ArrowRight', 'Tab', 'Enter', ' ', 'a']) expect(listAfterKey(list, 'sync', key)).toBeNull()
  })

  test('a current item that is not in the list moves nowhere', () => {
    expect(listAfterKey(['general', 'about'] as const, 'account' as never, 'ArrowDown')).toBeNull()
  })
})

describe('what the native menu may offer (§8.2, F15)', () => {
  test('View › Back only while Settings shows; nothing while a sheet is open', () => {
    expect(menuStateFor(home, false)).toEqual({ settingsShowing: false, sheetOpen: false })
    expect(menuStateFor({ mode: 'settings', section: 'general', returnTo: 'home' }, false)).toEqual({ settingsShowing: true, sheetOpen: false })
    expect(menuStateFor({ mode: 'settings', section: 'account', returnTo: 'home' }, true)).toEqual({ settingsShowing: true, sheetOpen: true })
  })
})

describe('the first view (§7.8 "Which section opens")', () => {
  test('Home, unless the page was opened on a Settings section (?tab=, kept for the render tools)', () => {
    expect(initialMainView('')).toEqual(home)
    expect(initialMainView('?window=main&platform=macos')).toEqual(home)
    expect(initialMainView('?window=main&platform=macos&tab=account')).toEqual({ mode: 'settings', section: 'account', returnTo: 'home' })
    expect(initialMainView('?tab=nonsense')).toEqual(home)
  })

  test('?section=review or trash opens that section (kept for the render tools, like ?tab=); a Settings tab wins; nonsense is Home', () => {
    expect(initialMainView('?window=main&platform=macos&section=review')).toEqual(review)
    expect(initialMainView('?section=trash')).toEqual(trash)
    expect(initialMainView('?section=settings')).toEqual(home)
    expect(initialMainView('?section=review&tab=sync')).toEqual({ mode: 'settings', section: 'sync', returnTo: 'home' })
  })
})

describe('Home › status: the popover state in one row (§7.6, §6.3)', () => {
  // `sync_phase` as Rust sends it unless a test sets it: the phase outside a Finder phase, and never a Finder phase
  // itself (Rust takes Finder as ready for it), so under one it defaults to "synced" (#139 review P3-13).
  const snap = (over: Record<string, unknown> = {}) => ({
    phase: 'synced', paused: false, pending_changes: 0, reason: null,
    engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 940 },
    ...over,
    sync_phase: over.sync_phase ?? (String(over.phase ?? 'synced').startsWith('finder_') ? 'synced' : over.phase ?? 'synced'),
  }) as any

  test('the row follows sync_phase alone: syncing over an idle engine says Syncing, synced over a busy one says Up to date (#139 review P3-4)', () => {
    // A file moving with the engine idle (the pulse is up to a second behind): Rust says syncing.
    expect(homeStatus(snap({ phase: 'syncing', engine: { state: 'idle', files_remaining: 0, last_tick_ok_at: 940 } }), 1000)).toEqual({
      tone: 'busy', title: 'Syncing', detail: null, reason: null, action: null,
    })
    expect(homeStatus(snap({ phase: 'synced', engine: { state: 'syncing', files_remaining: 3, last_tick_ok_at: 940 } }), 1000)).toEqual({
      tone: 'ok', title: 'Up to date', detail: 'Last checked 1 min ago', reason: null, action: null,
    })
  })

  test('up to date, with when it last checked', () => {
    expect(homeStatus(snap(), 1000)).toEqual({ tone: 'ok', title: 'Up to date', detail: 'Last checked 1 min ago', reason: null, action: null })
  })

  test('syncing says how many files are left, in the singular for one', () => {
    expect(homeStatus(snap({ phase: 'syncing', engine: { state: 'syncing', files_remaining: 3, last_tick_ok_at: null } }), 1000)).toMatchObject({ tone: 'busy', title: 'Syncing', detail: '3 files left' })
    expect(homeStatus(snap({ phase: 'syncing', engine: { state: 'syncing', files_remaining: 1, last_tick_ok_at: null } }), 1000).detail).toBe('1 file left')
    expect(homeStatus(snap({ phase: 'syncing', engine: { state: 'syncing', files_remaining: null, last_tick_ok_at: null } }), 1000).detail).toBeNull()
  })

  test('paused, offline, error and storage full use the popover’s words; the actions are the ones that exist', () => {
    expect(homeStatus(snap({ phase: 'paused', pending_changes: 4 }), 1000)).toEqual({
      tone: 'neutral', title: 'Sync is paused', detail: '4 changes on this Mac are waiting. Nothing syncs until you resume.', reason: null, action: 'resume',
    })
    expect(homeStatus(snap({ phase: 'paused', pending_changes: 1 }), 1000).detail).toBe('1 change on this Mac is waiting. Nothing syncs until you resume.')
    expect(homeStatus(snap({ phase: 'paused', pending_changes: 0 }), 1000).detail).toBe('Nothing syncs until you resume.')
    expect(homeStatus(snap({ phase: 'offline' }), 1000)).toMatchObject({ tone: 'alert', title: 'Can’t reach Beebeeb', detail: 'Check your connection. Files on this Mac are unchanged.', action: null })
    expect(homeStatus(snap({ phase: 'error', reason: { code: 'state_db_locked', detail: null } }), 1000)).toMatchObject({
      tone: 'alert', title: 'Sync stopped', detail: 'Beebeeb couldn’t finish syncing. Files on this Mac are unchanged.', reason: 'reason: state_db_locked', action: null,
    })
    expect(homeStatus(snap({ phase: 'storage_full', pending_changes: 2 }), 1000)).toMatchObject({
      tone: 'alert', title: 'Your storage is full', detail: '2 changes can’t upload until you free up space or add storage. They stay on this Mac.', action: 'add_storage',
    })
  })

  test('a Finder phase says the sync state under it: the Finder row says the Finder part', () => {
    expect(homeStatus(snap({ phase: 'finder_failed', sync_phase: 'synced' }), 1000).title).toBe('Up to date')
    expect(homeStatus(snap({ phase: 'finder_adding', sync_phase: 'syncing', engine: { state: 'syncing', files_remaining: 2, last_tick_ok_at: null } }), 1000)).toMatchObject({ title: 'Syncing', detail: '2 files left' })
  })

  test('last checked: just now, minutes, hours, and nothing when it is unknown', () => {
    expect(lastCheckedLine(995, 1000)).toBe('Last checked just now')
    expect(lastCheckedLine(1000 - 125, 1000)).toBe('Last checked 2 min ago')
    expect(lastCheckedLine(1000 - 3 * 3600, 1000)).toBe('Last checked 3 h ago')
    expect(lastCheckedLine(null, 1000)).toBeNull()
    expect(lastCheckedLine(2000, 1000)).toBe('Last checked just now')
  })
})

describe('connectMainNavigation: Rust stores one request and pokes the window; the window pulls it', () => {
  const { connectMainNavigation, MAIN_NAVIGATE_EVENT } = require('../src/mainWindowModel')
  const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve() }

  test('drains once when the listener lands (a window that was built for the request), then on every poke', async () => {
    let poke: () => void = () => {}
    const events: string[] = []
    const pending: unknown[] = [{ to: 'settings', section: 'about' }, null, { to: 'back' }]
    const applied: unknown[] = []
    const stop = connectMainNavigation(
      (event: string, callback: () => void) => { events.push(event); poke = callback; return Promise.resolve(() => {}) },
      async () => ({ ok: true, value: pending.shift() ?? null }),
      (nav: unknown) => applied.push(nav),
    )
    await flush()
    expect(events).toEqual([MAIN_NAVIGATE_EVENT])
    expect(applied).toEqual([{ to: 'settings', section: 'about' }])
    poke()
    await flush()
    expect(applied).toHaveLength(1) // nothing pending: nothing applied
    poke()
    await flush()
    expect(applied).toEqual([{ to: 'settings', section: 'about' }, { to: 'back' }])
    stop()
  })

  test('a failed pull or a request it cannot read applies nothing; after stop, a late listener is removed at once', async () => {
    const applied: unknown[] = []
    let removed = 0
    let land: (u: () => void) => void = () => {}
    const stop = connectMainNavigation(
      () => new Promise<() => void>((resolve) => { land = resolve }),
      async () => ({ ok: false, reason: 'x' }),
      (nav: unknown) => applied.push(nav),
    )
    stop()
    land(() => { removed += 1 })
    await flush()
    expect(removed).toBe(1)
    expect(applied).toEqual([])
    const stop2 = connectMainNavigation(
      () => Promise.resolve(() => {}),
      async () => ({ ok: true, value: { to: 'files' } }),
      (nav: unknown) => applied.push(nav),
    )
    await flush()
    expect(applied).toEqual([])
    stop2()
  })
})

describe('Home › status under a Finder phase: the sync part comes from Rust (#134 thread 2, task 2179)', () => {
  // The backend's `popover_phase` puts a Finder phase above paused, storage full, offline and error. Rust also sends
  // `sync_phase`, the same reducer with the Finder setup taken as ready, so Home's row says the sync part beside the
  // Finder row and this side keeps no copy of the precedence (task 2179 item 5; the Rust test
  // `the_sync_phase_is_the_phase_with_the_finder_taken_as_ready` walks every case).
  const finder = (over: Record<string, unknown> = {}) => ({
    phase: 'finder_user_disabled', sync_phase: 'synced', paused: false, storage_full: false, pending_changes: 0, reason: null,
    engine: { state: 'idle', files_remaining: 0, last_tick_ok_at: 940 },
    ...over,
  }) as any

  test('paused under a Finder phase is paused, with Resume sync', () => {
    expect(homeStatus(finder({ sync_phase: 'paused', paused: true, pending_changes: 2 }), 1000)).toEqual({
      tone: 'neutral', title: 'Sync is paused', detail: '2 changes on this Mac are waiting. Nothing syncs until you resume.', reason: null, action: 'resume',
    })
  })

  test('the engine saying paused is not the pause: only the live flag is, so a read right after a resume says so (item 3)', () => {
    // A snapshot read in the gap after a resume: the flag is off, the engine's last event still says paused. Rust's
    // reducer takes the flag only (`sync_phase` synced); before, this row inferred a pause from the engine.
    const gap = finder({ phase: 'finder_missing', sync_phase: 'synced', paused: false, engine: { state: 'paused', files_remaining: 0, last_tick_ok_at: 940 } })
    expect(homeStatus(gap, 1000)).toMatchObject({ title: 'Up to date', action: null })
  })

  test('an error with the auth reason is not "Sync stopped": the backend maps it to online (item 4)', () => {
    // engine_status `connectivity`: error + auth is the server answering 401, so Rust's sync part is synced.
    const auth = finder({ sync_phase: 'synced', reason: { code: 'auth', detail: null }, engine: { state: 'error', files_remaining: 0, last_tick_ok_at: 940 } })
    expect(homeStatus(auth, 1000).title).toBe('Up to date')
    const real = finder({ sync_phase: 'error', reason: { code: 'state_db_locked', detail: null }, engine: { state: 'error', files_remaining: 0, last_tick_ok_at: null } })
    expect(homeStatus(real, 1000)).toMatchObject({ title: 'Sync stopped', reason: 'reason: state_db_locked' })
  })

  test('storage full, offline and error show under every Finder phase too', () => {
    for (const phase of ['finder_failed', 'finder_user_disabled', 'finder_adding', 'finder_missing']) {
      expect(homeStatus(finder({ phase, sync_phase: 'storage_full', pending_changes: 1 }), 1000)).toMatchObject({ title: 'Your storage is full', action: 'add_storage' })
      expect(homeStatus(finder({ phase, sync_phase: 'offline' }), 1000)).toMatchObject({ title: 'Can’t reach Beebeeb' })
      expect(homeStatus(finder({ phase, sync_phase: 'error', reason: { code: 'state_db_locked', detail: null } }), 1000)).toMatchObject({ title: 'Sync stopped', reason: 'reason: state_db_locked' })
    }
  })

  test('the row follows sync_phase alone: the phase and the raw fields never decide it (item 5)', () => {
    expect(homeStatus(finder({ phase: 'finder_failed', sync_phase: 'storage_full', storage_full: false }), 1000).title).toBe('Your storage is full')
    expect(homeStatus(finder({ phase: 'paused', sync_phase: 'synced', paused: true }), 1000).title).toBe('Up to date')
    expect(homeStatus(finder({ sync_phase: 'syncing', engine: { state: 'syncing', files_remaining: 2, last_tick_ok_at: null } }), 1000)).toMatchObject({ title: 'Syncing', detail: '2 files left' })
  })

  test('main window code keeps no list of Finder phases (one precedence, in Rust)', () => {
    const source = require('node:fs').readFileSync(new URL('../src/mainWindowModel.ts', import.meta.url), 'utf8')
    expect(source).not.toMatch(/'finder_(failed|user_disabled|adding|missing)'/)
  })
})

describe('SnapshotReader: one read in flight, ticks skipped, the newest result applied (auto-review thread 1)', () => {
  const { SnapshotReader } = require('../src/mainWindowModel')
  const deferred = () => {
    let resolve: (v: unknown) => void = () => {}
    const promise = new Promise((r) => { resolve = r })
    return { promise, resolve }
  }
  const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve() }
  /** The caller's failure shape: what a read that threw, rejected or timed out gives. */
  const OPTIONS = { failure: (cause: string, error?: unknown) => ({ ok: false, cause, error }), watchdogMs: 60_000 }
  afterEach(() => jest.useRealTimers())

  test('a read slower than the tick still lands: ticks while it is in flight start nothing', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
    reader.resume()
    reader.tick()
    reader.tick()
    reader.tick()
    expect(reads).toHaveLength(1)
    reads[0].resolve({ ok: true, value: 'first' })
    await settle()
    expect(applied).toEqual([{ ok: true, value: 'first' }])
    reader.tick()
    expect(reads).toHaveLength(2)
  })

  test('a refresh after an outcome wins over a read started before it, whichever finishes first', async () => {
    for (const order of ['refresh first', 'older first'] as const) {
      const reads: Array<ReturnType<typeof deferred>> = []
      const applied: unknown[] = []
      const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
      reader.resume()
      reader.tick()
      reader.refresh() // the sign-out's outcome: read again now, even with one in flight
      expect(reads).toHaveLength(2)
      const afterOutcome = () => reads[1].resolve({ ok: true, value: 'after the outcome' })
      const beforeOutcome = () => reads[0].resolve({ ok: true, value: 'before the outcome' })
      for (const finish of order === 'refresh first' ? [afterOutcome, beforeOutcome] : [beforeOutcome, afterOutcome]) {
        finish()
        await settle()
      }
      // The state from before the outcome is never shown, not even for a moment.
      expect({ order, applied }).toEqual({ order, applied: [{ ok: true, value: 'after the outcome' }] })
    }
  })

  test('a failure is applied like a result, and a later read replaces it', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
    reader.resume()
    reader.tick()
    reads[0].resolve({ ok: false, reason: 'x' })
    await settle()
    reader.tick()
    reads[1].resolve({ ok: true, value: 'later' })
    await settle()
    expect(applied).toEqual([{ ok: false, reason: 'x' }, { ok: true, value: 'later' }])
  })

  test('stopped (the window unmounted) applies nothing; resumed (StrictMode remount) applies the read in flight', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
    reader.resume()
    reader.tick()
    reader.stop()
    reader.resume()
    reader.tick() // still in flight: skipped
    expect(reads).toHaveLength(1)
    reads[0].resolve({ ok: true, value: 'landed' })
    await settle()
    expect(applied).toEqual([{ ok: true, value: 'landed' }])
    // Unmounted with a read in flight: it finishes and nothing is applied, and no tick starts another.
    reader.refresh()
    reader.stop()
    reads[1].resolve({ ok: true, value: 'after unmount' })
    await settle()
    reader.tick()
    expect(applied).toEqual([{ ok: true, value: 'landed' }])
    expect(reads).toHaveLength(2)
  })

  // ── task 2179 items 2, 6, 7 and 9 ──

  test('a read that never answers is given up at the watchdog: its failure applies and the next tick reads again (item 6)', async () => {
    jest.useFakeTimers()
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: any[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), { ...OPTIONS, watchdogMs: 15_000 })
    reader.start(5000)
    expect(reads).toHaveLength(1)
    jest.advanceTimersByTime(14_999)
    await settle()
    expect(applied).toEqual([])
    jest.advanceTimersByTime(1)
    await settle()
    expect(applied).toEqual([{ ok: false, cause: 'timed out', error: undefined }])
    jest.advanceTimersByTime(5000)
    await settle()
    expect(reads).toHaveLength(2) // the hung read no longer holds the ticks
    reads[0].resolve({ ok: true, value: 'too late' })
    await settle()
    expect(applied).toHaveLength(1) // a given-up read applies nothing when it finally answers
    reader.stop()
  })

  test('only the current epoch\'s read holds the ticks: a read dropped by a refresh does not (item 6)', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
    reader.resume()
    reader.tick() // read 0 hangs
    void reader.refresh() // read 1, the new epoch
    reads[1].resolve({ ok: true, value: 'after' })
    await settle()
    reader.tick()
    expect(reads).toHaveLength(3) // read 0 is still unanswered, and does not block
    expect(applied).toEqual([{ ok: true, value: 'after' }])
  })

  test('a read() that throws fails like one that rejected, with the caller\'s failure shape, and never wedges the reader (item 6)', async () => {
    const applied: unknown[] = []
    let throws = true
    const error = new Error('bridge gone')
    const reader = new SnapshotReader(
      () => { if (throws) throw error; return Promise.reject(error) },
      (r: unknown) => applied.push(r),
      OPTIONS,
    )
    reader.resume()
    reader.tick()
    await settle()
    throws = false
    reader.tick()
    await settle()
    expect(applied).toEqual([{ ok: false, cause: 'threw', error }, { ok: false, cause: 'rejected', error }])
  })

  test('the next tick is scheduled when a read finishes, never on a fixed clock (item 6)', async () => {
    jest.useFakeTimers()
    const started: number[] = []
    let clock = 0
    const reader = new SnapshotReader(
      () => { started.push(clock); return new Promise((resolve) => setTimeout(() => resolve({ ok: true }), 8000)) },
      () => {},
      OPTIONS,
    )
    reader.start(5000)
    for (let i = 0; i < 30; i++) {
      clock += 1000
      jest.advanceTimersByTime(1000)
      await settle()
    }
    // An 8 s read, then 5 s: reads at 0, 13 and 26 s, not every 5 s.
    expect(started).toEqual([0, 13_000, 26_000])
    reader.stop()
  })

  test('retry joins the read in flight: five "Try again" clicks, one read (item 7)', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, () => {}, OPTIONS)
    reader.resume()
    const clicks = [reader.retry(), reader.retry(), reader.retry(), reader.retry(), reader.retry()]
    expect(reads).toHaveLength(1)
    let done = 0
    for (const click of clicks) void click.then(() => { done += 1 })
    reads[0].resolve({ ok: false, reason: 'x' })
    await settle()
    expect(done).toBe(5)
    void reader.retry()
    expect(reads).toHaveLength(2) // nothing in flight: a new read
  })

  test('refresh resolves once its own result is applied, to whether it was: a superseded or stopped read resolves false (item 2; #139 review P2-2)', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const events: string[] = []
    const reader = new SnapshotReader(
      () => { const d = deferred(); reads.push(d); return d.promise },
      (r: any) => events.push(`applied ${r.value}`),
      OPTIONS,
    )
    reader.resume()
    const first = reader.refresh().then((applied: boolean) => events.push(`first resolved ${applied}`))
    const second = reader.refresh().then((applied: boolean) => events.push(`second resolved ${applied}`))
    reads[1].resolve({ ok: true, value: 'second' })
    await second
    expect(events).toEqual(['applied second', 'second resolved true'])
    reads[0].resolve({ ok: true, value: 'first' }) // superseded: an older epoch
    await first
    expect(events).toEqual(['applied second', 'second resolved true', 'first resolved false'])
    const third = reader.refresh()
    reader.stop()
    reads[2].resolve({ ok: true, value: 'third' })
    expect(await third).toBe(false) // nothing applied after the window unmounted
    expect(events).toHaveLength(3)
  })

  test('discard (stop for a reader that is reused): a read still out never lands after a resume, and does not hold back the next lifetime\'s first tick (review of #142)', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), OPTIONS)
    reader.resume()
    reader.tick()
    reader.discard() // the vault locked, or the account was switched
    reader.resume()
    reader.tick() // the new lifetime reads at once: the discarded read is not "in flight" for it
    expect(reads).toHaveLength(2)
    reads[0].resolve({ ok: true, value: 'old lifetime' })
    await settle()
    expect(applied).toEqual([])
    reads[1].resolve({ ok: true, value: 'new lifetime' })
    await settle()
    expect(applied).toEqual([{ ok: true, value: 'new lifetime' }])
    // The late answer of the old lifetime did not corrupt the count of reads in flight: the next tick starts a read.
    reader.tick()
    expect(reads).toHaveLength(3)
  })

  test('discard with nothing out, and twice, is harmless; a refresh after it reads', async () => {
    const reads: Array<ReturnType<typeof deferred>> = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, () => {}, OPTIONS)
    reader.discard()
    reader.discard()
    reader.tick()
    expect(reads).toHaveLength(0) // stopped until resumed
    reader.resume()
    reader.refresh()
    expect(reads).toHaveLength(1)
  })

  test('a refresh whose read failed or timed out resolves false: the window never treats it as shown (#139 review P2-2)', async () => {
    jest.useFakeTimers()
    const reads: Array<ReturnType<typeof deferred>> = []
    const applied: unknown[] = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, (r: unknown) => applied.push(r), { ...OPTIONS, watchdogMs: 1000 })
    reader.resume()
    const failed = reader.refresh()
    reads[0].resolve({ ok: false, reason: 'offline' })
    expect(await failed).toBe(false)
    const silent = reader.refresh()
    jest.advanceTimersByTime(1000)
    // Raced, so a reader without its watchdog fails here at once instead of hanging the run on the fake clock.
    expect(await Promise.race([silent, settle().then(() => 'still pending')])).toBe(false)
    expect(applied).toEqual([{ ok: false, reason: 'offline' }, { ok: false, cause: 'timed out', error: undefined }])
    const fine = reader.refresh()
    reads[2].resolve({ ok: true, value: 'x' })
    expect(await fine).toBe(true)
  })

  test('a result that throws while it is applied still resolves the refresh and schedules the next tick (#139 review P3-6)', async () => {
    jest.useFakeTimers()
    const reads: Array<ReturnType<typeof deferred>> = []
    const errors: unknown[] = []
    const realError = console.error
    console.error = (...args: unknown[]) => { errors.push(args) }
    try {
      const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, () => { throw new Error('render bug') }, OPTIONS)
      reader.start(5000)
      reads[0].resolve({ ok: true, value: 'x' })
      await settle()
      expect(errors).toHaveLength(1)
      const refresh = reader.refresh()
      reads[1].resolve({ ok: true, value: 'y' })
      expect(await refresh).toBe(false)
      jest.advanceTimersByTime(5000)
      expect(reads).toHaveLength(3) // the tick after it still came
      reader.stop()
    } finally {
      console.error = realError
    }
  })

  test('retry joins a read in flight only while it is young: an older one gets a fresh read beside it (#139 review P3-7)', async () => {
    let clock = 0
    const reads: Array<ReturnType<typeof deferred>> = []
    const reader = new SnapshotReader(() => { const d = deferred(); reads.push(d); return d.promise }, () => {}, { ...OPTIONS, joinMs: 2000, now: () => clock })
    reader.resume()
    void reader.retry()
    clock = 1999
    void reader.retry()
    expect(reads).toHaveLength(1)
    clock = 2000
    const fresh = reader.retry()
    expect(reads).toHaveLength(2)
    reads[1].resolve({ ok: true, value: 'fresh' })
    expect(await fresh).toBe(true)
  })
})
