/**
 * The pure decisions of the macOS Settings window (task 1683 slice 4, spec section 5).
 * Each test pins one thing the window says or does; the mutation list in the task Notes names
 * the assertion that went red for each.
 */
import { readFileSync } from 'node:fs'
import { describe, expect, test } from 'bun:test'
import type { VaultItem } from '../src/desktopApi'
import type { FinderSetupView } from '../src/finderSetup'
import { FINDER_REASON_COPY } from '../src/finderSetupCopy'
import { DESKTOP_NOTIFICATION_PREF_META } from '../src/windows/notificationPreferenceMeta'
import {
  accountInitial,
  DEFAULT_SETTINGS_TAB,
  finderHint,
  finderRow,
  HELP_URL,
  keepCountLabel,
  keepOnMac,
  MONO_REASON_MAX,
  monoReason,
  NOTIFICATION_ROWS,
  parseSettingsTab,
  planLine,
  REPAIR_BODY,
  REPAIR_TITLE,
  repairNote,
  SETTINGS_TABS,
  settingsTabFromSearch,
  SPEED_PRESETS_KBPS,
  speedLabel,
  speedOptions,
  storageLine,
  STORAGE_RED_PERCENT,
  tabAfterKey,
  updateRow,
} from '../src/macSettingsModel'

describe('tabs', () => {
  test('the window has exactly the four tabs of the spec, in order', () => {
    expect(SETTINGS_TABS.map((t) => t.label)).toEqual(['General', 'Account', 'Sync', 'About'])
    expect(DEFAULT_SETTINGS_TAB).toBe('general')
  })

  test('?tab= deep-links a tab and anything else opens General', () => {
    expect(parseSettingsTab('sync')).toBe('sync')
    expect(parseSettingsTab('SYNC')).toBeNull()
    expect(parseSettingsTab(null)).toBeNull()
    expect(settingsTabFromSearch('?window=settings-v2&tab=about')).toBe('about')
    expect(settingsTabFromSearch('?tab=status')).toBe('general')
    expect(settingsTabFromSearch('')).toBe('general')
  })

  test('arrow keys move and wrap, Home and End jump, other keys are left alone', () => {
    expect(tabAfterKey('general', 'ArrowRight')).toBe('account')
    expect(tabAfterKey('about', 'ArrowRight')).toBe('general')
    expect(tabAfterKey('general', 'ArrowLeft')).toBe('about')
    expect(tabAfterKey('sync', 'ArrowLeft')).toBe('account')
    expect(tabAfterKey('sync', 'Home')).toBe('general')
    expect(tabAfterKey('sync', 'End')).toBe('about')
    expect(tabAfterKey('sync', 'Enter')).toBeNull()
    expect(tabAfterKey('sync', 'ArrowDown')).toBeNull()
  })
})

describe('the mono reason line', () => {
  test('is `reason: <category>`, one line of at most 30 characters, cut with an ellipsis', () => {
    expect(monoReason('timeout')).toBe('reason: timeout')
    expect(MONO_REASON_MAX).toBe(30)
    const long = monoReason('x'.repeat(60))
    expect(long).toHaveLength(30)
    expect(long!.endsWith('…')).toBe(true)
    // Exactly 30 is kept whole: 8 characters of "reason: " plus 22.
    expect(monoReason('y'.repeat(22))).toBe(`reason: ${'y'.repeat(22)}`)
    expect(monoReason('y'.repeat(23))).toHaveLength(30)
  })

  test('nothing to say means no line at all', () => {
    expect(monoReason(null)).toBeNull()
    expect(monoReason(undefined)).toBeNull()
    expect(monoReason('   ')).toBeNull()
  })
})

describe('Beebeeb in Finder (spec 2026-10-06)', () => {
  const v = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
    setup: 'missing', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null, ...over,
  })

  test('one row per reconciler state, and never a "missing" row with an add button (R5)', () => {
    expect(finderRow(null)).toEqual({ kind: 'loading' })
    expect(finderRow(null, true)).toEqual({ kind: 'unavailable' })
    expect(finderRow(v({ setup: 'ready' }))).toEqual({ kind: 'added' })
    expect(finderRow(v({ setup: 'adding' }))).toEqual({ kind: 'adding' })
    expect(finderRow(v({ setup: 'missing' }))).toEqual({ kind: 'adding' })
    expect(finderRow(v({ setup: 'failed', reason: 'folder_taken' }))).toEqual({
      kind: 'notice', tone: 'alert', reason: 'folder_taken',
      sentence: FINDER_REASON_COPY.folder_taken.sentence, action: 'copy_details', actionLabel: 'Copy details',
    })
    expect(finderRow(v({ setup: 'user_disabled', reason: 'user_disabled' }))).toMatchObject({ kind: 'notice', tone: 'status', action: 'open_system_settings' })
  })

  test('a failure without a reason is the "unknown" notice, not a silent row', () => {
    expect(finderRow(v({ setup: 'failed', reason: null }))).toMatchObject({ kind: 'notice', tone: 'alert', reason: 'unknown', action: 'try_again' })
  })

  test('a view that is there is always shown: loadFailed only speaks for "no view"', () => {
    expect(finderRow(v({ setup: 'ready' }), true)).toEqual({ kind: 'added' })
  })

  test('the hint claims the vault is in Finder only once it is', () => {
    expect(finderHint({ kind: 'added' })).toBe('Your vault appears under Locations in Finder.')
    expect(finderHint({ kind: 'adding' })).toBe('Adding Beebeeb to Finder…')
    expect(finderHint({ kind: 'loading' })).toBe('')
    expect(finderHint({ kind: 'unavailable' })).toBe('Couldn’t check Finder.')
    expect(finderHint(finderRow(v({ setup: 'failed', reason: 'timeout' })))).toBe('')
  })

  test('no row, hint or notice ever offers to add or install by hand (R5)', () => {
    const views = [v(), v({ setup: 'adding' }), v({ setup: 'ready' }), ...Object.keys(FINDER_REASON_COPY).map((reason) => v({ setup: 'failed', reason: reason as never }))]
    for (const view of views) {
      const row = finderRow(view)
      expect(JSON.stringify(row)).not.toMatch(/Add to Finder|Install/)
      expect(finderHint(row)).not.toMatch(/Add it|Add to Finder|Install/)
    }
  })

  test('the row carries no title and no raw error: one sentence and one action (spec §6.2)', () => {
    const row = finderRow(v({ setup: 'failed', reason: 'timeout', last_failure: { reason: 'timeout', domain: 'NSFileProviderErrorDomain', code: -1001, at: 5 } }))
    expect(Object.keys(row).sort()).toEqual(['action', 'actionLabel', 'kind', 'reason', 'sentence', 'tone'])
    expect(JSON.stringify(row)).not.toContain('NSFileProviderErrorDomain')
  })

  test('Repair says what it does, names the login item, and that Beebeeb comes back by itself', () => {
    expect(REPAIR_TITLE).toBe('Repair Beebeeb in Finder?')
    expect(REPAIR_BODY).toBe(
      'Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.',
    )
  })

  test('the Repair body is the one the design artefact draws, byte for byte (design before code)', () => {
    const html = readFileSync(new URL('../design/hifi/macos-settings-dialogs.html', import.meta.url), 'utf8')
    expect(html).toContain(`<p class="sheet-copy">${REPAIR_BODY}</p>`)
    expect(html).toContain(`body “${REPAIR_BODY}”`)
    // The old promise (a manual add-back) is gone from the code and from the drawing.
    expect(REPAIR_BODY).not.toContain('You can add it back afterwards')
  })

  test('after a repair: one neutral line, singular and plural, warnings appended, nothing when empty', () => {
    expect(repairNote({ pending_operations_preserved: 0, warnings: [] })).toBeNull()
    expect(repairNote({ pending_operations_preserved: 1, warnings: [] })).toBe('1 change waiting to upload was kept.')
    expect(repairNote({ pending_operations_preserved: 3, warnings: [] })).toBe('3 changes waiting to upload were kept.')
    expect(repairNote({ pending_operations_preserved: 0, warnings: ['  ', 'Could not remove a file.'] })).toBe('Could not remove a file.')
    expect(repairNote({ pending_operations_preserved: 2, warnings: ['Could not remove a file.'] })).toBe(
      '2 changes waiting to upload were kept. Could not remove a file.',
    )
  })
})

describe('Keep on this Mac', () => {
  const folder = (id: string, name: string, over: Partial<VaultItem> = {}): VaultItem => ({
    id,
    name,
    is_folder: true,
    excluded: false,
    size_bytes: 0,
    file_count: 0,
    on_disk_bytes: 0,
    children: [],
    ...over,
  })
  const file = (id: string): VaultItem => ({ ...folder(id, id), is_folder: false })

  test('loading and failed are their own states', () => {
    expect(keepOnMac(null, false)).toEqual({ kind: 'loading' })
    expect(keepOnMac(null, true)).toEqual({ kind: 'failed' })
  })

  test('counts the folders the backend says are kept, at any depth, and ignores files', () => {
    const tree = [
      folder('a', 'Photos', { pinned: true, children: [folder('a1', 'Trips', { pinned: true }), file('f1')] }),
      folder('b', 'Work', { pinned: false }),
    ]
    const result = keepOnMac(tree, false)
    expect(result.kind).toBe('ready')
    if (result.kind !== 'ready') return
    expect(result.pinned).toBe(2)
    expect(result.folders.map((f) => [f.name, f.where, f.pinned])).toEqual([
      ['Photos', '', true],
      ['Trips', 'Photos', true],
      ['Work', '', false],
    ])
  })

  test('never says "0 folders" when the backend sends no pin state at all (today\'s list_vault_folders)', () => {
    const tree = [folder('a', 'Photos'), folder('b', 'Work', { children: [folder('c', 'Deep')] })]
    expect(keepOnMac(tree, false)).toEqual({ kind: 'unreported' })
  })

  test('an empty vault is a real, empty answer', () => {
    expect(keepOnMac([], false)).toEqual({ kind: 'ready', folders: [], pinned: 0 })
  })

  test('the count reads in plain words', () => {
    expect(keepCountLabel(0)).toBe('No folders')
    expect(keepCountLabel(1)).toBe('1 folder')
    expect(keepCountLabel(3)).toBe('3 folders')
  })
})

describe('speed limits', () => {
  test('0 is "No limit", Mbps from 1000 kbps up, kbps below', () => {
    expect(speedLabel(0)).toBe('No limit')
    expect(speedLabel(-5)).toBe('No limit')
    expect(speedLabel(1000)).toBe('1 Mbps')
    expect(speedLabel(2500)).toBe('2.5 Mbps')
    expect(speedLabel(100000)).toBe('100 Mbps')
    expect(speedLabel(500)).toBe('500 kbps')
  })

  test('the options are the presets, in order, starting with no limit', () => {
    const options = speedOptions(0)
    expect(options.map((o) => o.value)).toEqual([...SPEED_PRESETS_KBPS])
    expect(options[0]).toEqual({ value: 0, label: 'No limit' })
    expect(options.map((o) => o.label)).toEqual(['No limit', '1 Mbps', '2 Mbps', '5 Mbps', '10 Mbps', '20 Mbps', '50 Mbps', '100 Mbps'])
  })

  test('a limit an older version saved stays selectable, in order, and is not duplicated', () => {
    expect(speedOptions(3000).map((o) => o.value)).toEqual([0, 1000, 2000, 3000, 5000, 10000, 20000, 50000, 100000])
    expect(speedOptions(5000).map((o) => o.value)).toEqual([...SPEED_PRESETS_KBPS])
    expect(speedOptions(Number.NaN).map((o) => o.value)).toEqual([...SPEED_PRESETS_KBPS])
  })
})

describe('account', () => {
  test('the avatar initial is the first letter or digit, upper-cased, with a brand fallback', () => {
    expect(accountInitial('sam@example.eu')).toBe('S')
    expect(accountInitial('  émile@example.eu')).toBe('É')
    expect(accountInitial('_.7up@example.eu')).toBe('7')
    expect(accountInitial(null)).toBe('B')
    expect(accountInitial('...')).toBe('B')
  })

  test('the plan line says renews for a live plan, ends for a cancelled one, and invents no date', () => {
    const live = { plan: 'basic', status: 'active', current_period_end: '2026-10-14T12:00:00Z' }
    expect(planLine(live)).toBe('Basic plan · renews 14 Oct 2026')
    expect(planLine({ ...live, status: 'trialing' })).toBe('Basic plan · renews 14 Oct 2026')
    expect(planLine({ ...live, status: 'canceled' })).toBe('Basic plan · ends 14 Oct 2026')
    expect(planLine({ ...live, current_period_end: null })).toBe('Basic plan')
    expect(planLine({ plan: 'free', status: 'active', current_period_end: null })).toBe('Free plan')
    expect(planLine(null)).toBeNull()
    expect(planLine({ plan: '', status: 'active', current_period_end: null })).toBeNull()
  })

  test('storage: the Mac\'s number format, a percentage, red from 90 % and not before', () => {
    const line = storageLine({ used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000 }, 'nl-NL')
    expect(line).toEqual({ label: '84,3 GB of 200 GB', percent: 42.15, red: false })
    expect(STORAGE_RED_PERCENT).toBe(90)
    expect(storageLine({ used_bytes: 180_000_000_000, quota_bytes: 200_000_000_000 }, 'en')!.red).toBe(true)
    expect(storageLine({ used_bytes: 179_000_000_000, quota_bytes: 200_000_000_000 }, 'en')!.red).toBe(false)
    expect(storageLine({ used_bytes: 5, quota_bytes: 0 })).toBeNull()
    expect(storageLine(null)).toBeNull()
  })
})

describe('General', () => {
  test('"Tell me about" uses the three existing preference keys, unchanged, in the spec\'s words', () => {
    expect(NOTIFICATION_ROWS.map((r) => r.key)).toEqual(DESKTOP_NOTIFICATION_PREF_META.map((m) => m.key))
    expect(NOTIFICATION_ROWS.map((r) => r.label)).toEqual(['Conflicts', 'Sync finished', 'Local cache almost full'])
    expect(NOTIFICATION_ROWS[0].hint).toBe('When two versions of a file need a decision.')
  })
})

describe('About', () => {
  test('the manual update check answers inline, in plain words, for every state', () => {
    expect(updateRow({ kind: 'idle' }, '0.8.6')).toEqual({ hint: 'Version 0.8.6', action: 'check' })
    expect(updateRow({ kind: 'idle' }, null)).toEqual({ hint: '', action: 'check' })
    expect(updateRow({ kind: 'checking' }, '0.8.6')).toEqual({ hint: 'Checking…', action: 'checking' })
    expect(updateRow({ kind: 'up_to_date', currentVersion: '0.8.6', channel: 'stable' }, '0.8.6')).toEqual({
      hint: 'Version 0.8.6 is the latest.',
      action: 'check',
    })
    expect(updateRow({ kind: 'update_available', currentVersion: '0.8.6', channel: 'stable', version: '0.8.7' }, '0.8.6')).toEqual({
      hint: 'Version 0.8.7 is available.',
      action: 'install',
    })
    const error = updateRow({ kind: 'error', reason: 'Could not reach the stable update manifest: connect' }, '0.8.6')
    expect(error.action).toBe('check')
    expect(error.hint).toBe('Couldn’t check for updates. Try again in a moment.')
    expect(error.hint).not.toContain('manifest')
  })

  test('Get help opens the support page the Help menu already uses', () => {
    expect(HELP_URL).toBe('https://beebeeb.io/support')
  })
})
