/**
 * Task 1546 Codex round 2 — source-wiring checks for findings 2, 3 and 5
 * (mirrors the existing `conflictWindowNoPlaceholder.test.ts` /
 * `versionCenterReviewAction.test.ts` style: assert the real call sites wire
 * through the fixed decision points, not the old ones).
 *
 * Finding 3 — ConflictWindow.tsx must never trust the URL's `isText` flag
 * for anything past the very first paint: `open_conflict_window`'s caller in
 * VersionCenter.tsx always sets `isText: false` (only the daemon's own
 * auto-open path derives it correctly from the filename), so the ONLY
 * trustworthy value is `is_text` as returned by the backend's
 * `conflict_content_preview` response.
 *
 * Finding 2 / 5 — both the VersionCenter "Sign in again" review action and
 * the persistent auth-expired banner must route through the same forced
 * reauth flow (`forceReauth`), never `open_onboarding_window` alone.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const read = (rel: string) => readFileSync(join(import.meta.dir, '..', rel), 'utf8')

describe('ConflictWindow textness (finding 3)', () => {
  const src = read('src/ConflictWindow.tsx')

  test('the rendered isText comes from the backend-returned preview, not just the URL', () => {
    expect(src).toContain('preview?.is_text')
  })

  test('conflictContentPreview is called with no isText argument (the backend decides)', () => {
    // The OLD call site passed a second argument the daemon then trusted
    // blindly: `conflictContentPreview(fileId, isText)`.
    expect(src).not.toMatch(/conflictContentPreview\(\s*fileId\s*,/)
    expect(src).toMatch(/conflictContentPreview\(\s*fileId\s*\)/)
  })
})

describe('desktopApi.conflictContentPreview signature (finding 3)', () => {
  const src = read('src/desktopApi.ts')

  test('the wrapper no longer accepts an isText parameter', () => {
    expect(src).toMatch(/export function conflictContentPreview\(fileId: string\)/)
  })
})

describe('forced reauth wiring (findings 2 and 5)', () => {
  test('VersionCenter\'s "Sign in again" action routes through forceReauth, not open_onboarding_window directly', () => {
    const src = read('src/pages/VersionCenter.tsx')
    expect(src).toContain('forceReauth')
    // The exact bug: calling `open_onboarding_window` while the stale
    // session was still installed. `command<void>('open_onboarding_window')`
    // must no longer appear as its own call in this file.
    expect(src).not.toContain("command<void>('open_onboarding_window')")
  })

  test('the persistent auth-expired banner exists and uses the same forced reauth flow', () => {
    const src = read('src/AuthExpiredBanner.tsx')
    expect(src).toContain('forceReauth')
    // Persistent: never auto-dismisses and the user can't dismiss it away
    // while still signed out — only a real sign-in (or a success) clears it.
    expect(src).toContain('durationMs: null')
    expect(src).toContain('dismissible: false')
  })

  test('sync_status reports auth_expired independently of logged_in', () => {
    const src = read('src-tauri/src/lib.rs')
    expect(src).toContain('"auth_expired": auth_expired')
  })
})
