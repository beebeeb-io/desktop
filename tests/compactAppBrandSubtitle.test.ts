/**
 * Guard for task 1546 finding 5: App.tsx (the compact app shell) is the SAME
 * component tree on macOS and Linux (main.tsx tags ?platform=macos for macOS
 * only, a CSS-class toggle — the JSX itself has no platform conditional), but
 * the sidebar brand subtitle was the hardcoded literal "macOS Drive". A
 * shipped Linux build (desktop-v0.1.0) showed "Beebeeb / macOS Drive".
 *
 * `brandSubtitleForPlatform` is the pure decision App.tsx renders through,
 * fed by the existing `desktop_platform` IPC command (same command
 * SyncFolder.tsx/Onboarding.tsx already call).
 */
import { describe, expect, test } from 'bun:test'
import { brandSubtitleForPlatform } from '../src/compactNavigation'

describe('compact app brand subtitle', () => {
  test('names the real OS on each platform', () => {
    expect(brandSubtitleForPlatform('macos')).toBe('macOS Drive')
    expect(brandSubtitleForPlatform('windows')).toBe('Windows Drive')
    expect(brandSubtitleForPlatform('linux')).toBe('Linux Drive')
  })

  test('falls back to a platform-neutral label before desktop_platform resolves', () => {
    expect(brandSubtitleForPlatform('unknown')).toBe('Drive')
  })
})
