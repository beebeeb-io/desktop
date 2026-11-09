/**
 * Desktop v2 M3b (B11): first run and the guide.
 * - File Explorer on Windows is the Cloud Files registration (`windows_shell_integration_state`), not macOS's Finder
 *   reconciler: registered → ready; a recorded error → failed with Windows' HRESULT; never tried or a try running → adding.
 * - The guide's steps: the file-menu tip is skipped when the list has no file (DW-0302's rule); Done ends after tip 5.
 * - A tip whose target sits at the bottom of the window (the gear, DW-0305) shows its card above the target.
 */
import { describe, expect, test } from 'bun:test'
import { explorerStatus, hresultOf, needsFirstRegistration, type ShellIntegrationView } from '../src/v2/onboarding/index'
import { GUIDE_LAST, GUIDE_SECTIONS, nextGuideStep } from '../src/v2/guide/GuideHost'
import { cardPlace } from '../src/v2/guide/Guide'

const view = (o: Partial<ShellIntegrationView>): ShellIntegrationView => ({
  installed: false, path: 'C:\\Users\\Anna\\Beebeeb', status: 'missing', last_error: null, last_attempt_at: null, reason_category: null, ...o,
})

describe('File Explorer step on Windows', () => {
  test('registered, failed, never tried', () => {
    expect(explorerStatus(view({ installed: true, status: 'installed' }))).toEqual({ status: 'ready', code: null })
    expect(explorerStatus(view({ status: 'error', last_error: 'register failed (0x8007017c)' }))).toEqual({ status: 'failed', code: '0x8007017C' })
    expect(explorerStatus(view({ status: 'missing' }))).toEqual({ status: 'adding', code: null })
    expect(explorerStatus(null)).toEqual({ status: 'adding', code: null })
  })

  test('a try that is running shows adding, not the last failure', () => {
    expect(explorerStatus(view({ status: 'error', last_error: 'x (0x8007017C)' }), true)).toEqual({ status: 'adding', code: null })
  })

  test('the HRESULT is shown only when the error carries one', () => {
    expect(hresultOf('The cloud sync provider failed (0x8007017C)')).toBe('0x8007017C')
    expect(hresultOf('no code here')).toBeNull()
    expect(hresultOf(null)).toBeNull()
  })

  test('setup registers a PC that never tried; never one that is registered or failed', () => {
    expect(needsFirstRegistration(view({ status: 'missing' }))).toBe(true)
    expect(needsFirstRegistration(view({ installed: true, status: 'installed' }))).toBe(false)
    expect(needsFirstRegistration(view({ status: 'error' }))).toBe(false)
    expect(needsFirstRegistration(null)).toBe(false)
  })
})

describe('the guide', () => {
  test('steps 0 to 5 in order with a file in the list; the end after 5', () => {
    const seen: (number | null)[] = []
    for (let s: number | null = 0; s != null; s = nextGuideStep(s, true)) seen.push(s)
    expect(seen).toEqual([0, 1, 2, 3, 4, 5])
    expect(GUIDE_LAST).toBe(5)
    expect(GUIDE_SECTIONS).toEqual(['files', 'files', 'files', 'shared', 'photos', 'files'])
  })

  test('no file in the list: the file-menu tip (step 2) is skipped', () => {
    expect(nextGuideStep(1, false)).toBe(3)
    expect(nextGuideStep(1, true)).toBe(2)
  })

  test('a target near the bottom puts the card above it; otherwise under it', () => {
    const view = { width: 1280, height: 800 }
    const gear = { top: 728, left: 180, width: 36, height: 36 }
    const placed = cardPlace(gear, 130, view)
    expect(placed.top + 130).toBeLessThanOrEqual(gear.top)
    const toolbar = { top: 40, left: 1180, width: 88, height: 32 }
    expect(cardPlace(toolbar, 130, view).top).toBe(toolbar.top + toolbar.height + 10)
  })
})

describe('review fixes (M3b self-review)', () => {
  test('a try Rust refused before recording anything shows failed with Try again, not "Adding" for ever', () => {
    expect(explorerStatus(view({ status: 'missing' }), false, 'could not create the folder (0x80070005)')).toEqual({ status: 'failed', code: '0x80070005' })
    expect(explorerStatus(view({ status: 'missing' }), false, 'session changed')).toEqual({ status: 'failed', code: null })
    expect(explorerStatus(view({ status: 'missing' }), true, 'session changed')).toEqual({ status: 'adding', code: null })
  })

  test('Show the guide again starts the guide in this window when storage refuses', async () => {
    const guide = await import('../src/v2/guide/Guide')
    const events: string[] = []
    const g = globalThis as Record<string, unknown>
    const before = { window: g.window, localStorage: g.localStorage }
    g.window = { dispatchEvent: (e: Event) => events.push(e.type) }
    g.localStorage = { getItem() { throw new Error('refused') }, setItem() { throw new Error('refused') }, removeItem() { throw new Error('refused') } }
    try {
      expect(guide.guideDue()).toBe(false)
      guide.startGuide()
      expect(events).toEqual([guide.GUIDE_EVENT])
      expect(guide.guideDue()).toBe(true)
      guide.markGuideSeen()
      expect(guide.guideDue()).toBe(false)
    } finally {
      g.window = before.window
      g.localStorage = before.localStorage
    }
  })

  // P1a (#271 review): a row says whether it is a file (`data-kind`, from item.isFolder); the name's extension decided it
  // before, which took a folder named `v1.2` for a file and missed `README`.
  test('the file-menu tip finds a file row or tile, and nothing in a list of folders', async () => {
    const { findTarget } = await import('../src/v2/guide/Guide')
    const row = (name: string, kind: 'file' | 'folder') => ({ name, kind })
    const g = globalThis as Record<string, unknown>
    const before = g.document
    const show = (rows: { name: string; kind: string }[]) => {
      g.document = {
        querySelectorAll: () => [],
        querySelector: (s: string) => (s === 'main [role="row"][data-kind="file"]' ? rows.find((r) => r.kind === 'file') ?? null : null),
      }
    }
    try {
      show([row('Documents', 'folder'), row('v1.2', 'folder')])
      expect(findTarget('file-menu')).toBeNull()
      const file = row('Budget 2026.xlsx', 'file')
      show([row('Documents', 'folder'), file])
      expect(findTarget('file-menu')).toBe(file as never)
      const readme = row('README', 'file')
      show([row('node.js', 'folder'), readme])
      expect(findTarget('file-menu')).toBe(readme as never)
    } finally {
      g.document = before
    }
  })
})
