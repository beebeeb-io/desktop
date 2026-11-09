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
