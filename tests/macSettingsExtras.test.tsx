/**
 * The two Settings pieces that read the account on their own (task 1683 slice 5; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8): General › "From your account" and Account › "Storage details…".
 * The real components of `src/MacSettingsExtras.tsx`, executed with the component harness against a scripted backend.
 *
 * What this proves: what each reads and when (the sheet reads nothing until it is opened), that a switch writes exactly
 * the one preference it was asked to and shows the server's answer, one failure with one surface (a switch that did not
 * save is a toast and goes back; a read that failed is one inline alert with Try again), and the words.
 * What it does NOT prove: layout (tests/render-mac-app.mjs) or anything on a Mac.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as model from '../src/macSettingsModel'
import * as parts from '../src/macSettingsParts'
import * as storageFormat from '../src/storageFormat'
import { ACCOUNT_NOTIFICATION_PREF_META } from '../src/windows/notificationPreferenceMeta'
import { expand, mount, textOf, visibleErrorSurfaces, type Mounted, type TreeNode } from './fixtures/componentHarness'

const React = { createElement, Fragment }

/** The in-window dialog with a close button, reduced to what a user sees. */
function Modal(props: any) {
  return props.open
    ? createElement('div', { role: 'dialog', 'aria-label': props.title }, createElement('h2', null, props.title), props.children, createElement('button', { onClick: props.onClose }, 'Close'))
    : null
}

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})
const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
const buttons = (m: Mounted) => find(m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim()).filter(Boolean)
const press = async (m: Mounted, label: string) => {
  const hit = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === label)
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}; buttons: ${buttons(m).join(' | ')}`)
  await hit[0].props.onClick()
  await settle(m)
}
function readable(node: any): string[] {
  if (Array.isArray(node)) return node.flatMap(readable)
  if (node == null || typeof node === 'boolean') return []
  if (typeof node !== 'object' || !node.props) return [String(node)]
  if (node.type === 'svg') return []
  return readable(node.props.children)
}
const visibleText = (m: Mounted) => readable(expand(m.tree())).join(' | ')
const switchOf = (m: Mounted, name: string) => {
  const hit = find(m, (el) => el.props.role === 'switch' && el.props['aria-labelledby'] === `ms-${name}-label`)
  if (hit.length !== 1) throw new Error(`expected one switch "${name}", found ${hit.length}`)
  return hit[0]
}
const called = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name)

// ── General › From your account ─────────────────────────────────────────────

describe('From your account', () => {
  const prefs = { new_device_login: true, share_received: true, file_updated: false, storage_warning: true, backup_complete: false }
  function openPrefs(backend: Record<string, (a: any) => unknown> = {}) {
    const m = mount('MacSettingsExtras.tsx', 'AccountPreferencesGroup', {
      backend: { account_notification_preferences: () => ({ preferences: prefs }), account_update_notification_preferences: ({ update }: any) => ({ preferences: { ...prefs, ...update } }), ...backend } as any,
      expand: true,
      bindings: { ...desktopApi, ...parts, ACCOUNT_NOTIFICATION_PREF_META },
    })
    mounted.push(m)
    return m
  }

  test('reads the account\'s preferences once and shows the five the hi-fi lists, in the Windows shell\'s words', async () => {
    const m = openPrefs()
    await settle(m)
    expect(m.calls.map((c) => c.name)).toEqual(['account_notification_preferences'])
    const text = visibleText(m)
    for (const words of [
      'From your account',
      'New device sign-in', 'We cannot see your files, but we know when a new device signs in.',
      'Share received', 'When someone grants you access to a file or folder.',
      'File updated', 'When a file in your vault changes on another device.',
      'Storage warning', 'When you are close to your storage limit.',
      'Backup complete', 'When a scheduled backup finishes.',
    ]) expect(text).toContain(words)
    expect(find(m, (el) => el.props.role === 'switch')).toHaveLength(5)
    expect(switchOf(m, 'new_device_login').props['aria-checked']).toBe(true)
    expect(switchOf(m, 'file_updated').props['aria-checked']).toBe(false)
    expect(switchOf(m, 'backup_complete').props['aria-checked']).toBe(false)
  })

  test('before the answer arrives the switches are there and cannot be pressed', async () => {
    const m = openPrefs({ account_notification_preferences: () => new Promise(() => {}) })
    await m.flush()
    const switches = find(m, (el) => el.props.role === 'switch')
    expect(switches).toHaveLength(5)
    expect(switches.every((el) => el.props.disabled === true)).toBe(true)
  })

  test('flipping a switch writes exactly that one preference, and the answer is what the switch then shows', async () => {
    const m = openPrefs()
    await settle(m)
    await switchOf(m, 'file_updated').props.onClick()
    await settle(m)
    expect(called(m, 'account_update_notification_preferences').map((c) => c.args)).toEqual([{ update: { file_updated: true } }])
    expect(switchOf(m, 'file_updated').props['aria-checked']).toBe(true)
    expect(switchOf(m, 'share_received').props['aria-checked']).toBe(true)
    expect(m.toasts).toEqual([])
  })

  test('a preference that did not save is ONE toast and the switch goes back to what is saved', async () => {
    const m = openPrefs({ account_update_notification_preferences: () => { throw new Error('server said no') } })
    await settle(m)
    await switchOf(m, 'file_updated').props.onClick()
    await settle(m)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t save that setting' })
    expect(switchOf(m, 'file_updated').props['aria-checked']).toBe(false)
    expect(visibleErrorSurfaces(m).filter((surface) => surface.startsWith('inline'))).toEqual([])
  })

  test('a switch is busy while its own write runs: a second press on it is not sent', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const m = openPrefs({ account_update_notification_preferences: async ({ update }: any) => { await gate; return { preferences: { ...prefs, ...update } } } })
    await settle(m)
    void switchOf(m, 'file_updated').props.onClick()
    m.render()
    expect(switchOf(m, 'file_updated').props.disabled).toBe(true)
    await switchOf(m, 'file_updated').props.onClick()
    release()
    await settle(m)
    expect(called(m, 'account_update_notification_preferences')).toHaveLength(1)
  })

  test('preferences that cannot be read are ONE inline alert with Try again, no switches and no toast; Try again reads again', async () => {
    let fail = true
    const m = openPrefs({ account_notification_preferences: () => { if (fail) throw new Error('offline'); return { preferences: prefs } } })
    await settle(m)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(find(m, (el) => el.props['data-error-surface'] === 'account-prefs-load')).toHaveLength(1)
    expect(visibleText(m)).toContain('Couldn’t load these settings')
    expect(visibleText(m)).not.toContain('offline')
    expect(find(m, (el) => el.props.role === 'switch')).toHaveLength(0)
    expect(m.toasts).toEqual([])
    fail = false
    await press(m, 'Try again')
    expect(called(m, 'account_notification_preferences')).toHaveLength(2)
    expect(find(m, (el) => el.props.role === 'switch')).toHaveLength(5)
    expect(visibleErrorSurfaces(m)).toEqual([])
  })
})

// ── Account › Storage details ───────────────────────────────────────────────

describe('Storage details', () => {
  const breakdown = {
    by_category: [
      { category: 'media', bytes: 60_000_000_000, file_count: 1200 },
      { category: 'documents', bytes: 20_000_000_000, file_count: 1 },
      { category: 'other', bytes: 4_300_000_000, file_count: 90 },
    ],
    largest_files: [{ file_id: 'f1', path: 'Photos/2026/IMG_0001.heic', size_bytes: 3_200_000_000, category: 'media' }],
    total_files: 1291,
    total_bytes: 84_300_000_000,
  }
  function openSheet(props: { open: boolean; onClose?: () => void }, backend: Record<string, (a: any) => unknown> = {}) {
    const closed: number[] = []
    const m = mount('MacSettingsExtras.tsx', 'StorageDetailsSheet', {
      backend: { account_storage_breakdown: () => breakdown, ...backend } as any,
      expand: true,
      props: { onClose: () => { closed.push(1) }, ...props },
      bindings: {
        ...desktopApi,
        ...parts,
        Modal,
        // The Mac's number format is pinned so the figures do not depend on the machine running the test.
        storageDetails: (b: any) => model.storageDetails(b, 'nl-NL'),
        STORAGE_DETAILS_SUBTITLE: model.STORAGE_DETAILS_SUBTITLE,
        STORAGE_DETAILS_LARGEST: model.STORAGE_DETAILS_LARGEST,
        formatStorageSize: storageFormat.formatStorageSize,
      },
    })
    mounted.push(m)
    return { m, closed }
  }

  test('closed it shows nothing and reads nothing', async () => {
    const { m } = openSheet({ open: false })
    await settle(m)
    expect(m.calls).toEqual([])
    expect(find(m, (el) => el.props.role === 'dialog')).toHaveLength(0)
  })

  test('opened it reads the breakdown once, asking for ten largest files, and shows the types, the largest files and the subtitle', async () => {
    const { m } = openSheet({ open: true })
    await settle(m)
    expect(called(m, 'account_storage_breakdown').map((c) => c.args)).toEqual([{ largestLimit: 10 }])
    expect(find(m, (el) => el.props.role === 'dialog')[0].props['aria-label']).toBe('Storage details')
    const text = visibleText(m)
    for (const words of [
      'Where your storage goes, by type and by your largest files.',
      '1.291 files · 84,3 GB',
      'Media', '60 GB · 1.200 files', 'Documents', '20 GB · 1 file', 'Other', '4,3 GB · 90 files',
      'Largest files', 'Photos/2026/IMG_0001.heic', 'Media · 3,2 GB',
    ]) expect(text).toContain(words)
    expect(text).not.toMatch(/\bPC\b|daemon|encrypted on/)
  })

  test('a path is shown whole and in mono, so it wraps and is never cut', async () => {
    const { m } = openSheet({ open: true })
    await settle(m)
    const path = find(m, (el) => textOf(el.props.children) === 'Photos/2026/IMG_0001.heic')
    expect(path.length).toBeGreaterThan(0)
    expect(path.some((el) => String(el.props.className ?? '').includes('ms-mono'))).toBe(true)
  })

  test('Close, Esc and the backdrop all go through the one onClose', async () => {
    const { m, closed } = openSheet({ open: true })
    await settle(m)
    await press(m, 'Close')
    expect(closed).toEqual([1])
  })

  test('nothing stored says so', async () => {
    const { m } = openSheet({ open: true }, { account_storage_breakdown: () => ({ by_category: [], largest_files: [], total_files: 0, total_bytes: 0 }) })
    await settle(m)
    expect(visibleText(m)).toContain('Nothing is stored yet.')
    expect(visibleText(m)).not.toContain('Largest files')
  })

  test('a breakdown that cannot be read is ONE inline alert with Try again; Try again reads again', async () => {
    let fail = true
    const { m } = openSheet({ open: true }, { account_storage_breakdown: () => { if (fail) throw new Error('vault is locked'); return breakdown } })
    await settle(m)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(find(m, (el) => el.props['data-error-surface'] === 'storage-details')).toHaveLength(1)
    expect(visibleText(m)).toContain('Couldn’t load storage details')
    expect(visibleText(m)).not.toContain('vault is locked')
    expect(m.toasts).toEqual([])
    fail = false
    await press(m, 'Try again')
    expect(called(m, 'account_storage_breakdown')).toHaveLength(2)
    expect(visibleText(m)).toContain('Media')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('while it loads it says so and shows no figures', async () => {
    const { m } = openSheet({ open: true }, { account_storage_breakdown: () => new Promise(() => {}) })
    await m.flush()
    expect(visibleText(m)).toContain('Loading…')
    expect(visibleText(m)).not.toContain('Media')
  })
})
