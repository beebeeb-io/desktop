import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { macRoots, reachableModules } from './fixtures/copyGuard'

/**
 * Task 2176 (lead decision 2026-10-24 22:47; two-surface spec §0 ruling 5): no surface a Mac can mount installs an update in
 * place or offers to restart into one. The behaviour is pinned where it runs (tests/macUpdateBanner.test.tsx, the About tests
 * of tests/macSettingsTabs.test.tsx, the Rust pins in `mac_update_tests`); this is the census that a NEW surface cannot slip
 * past them: the modules a Mac can reach that name an install command, or say "Restart to update", are exactly the known
 * ones, so a new one is a red test and a decision.
 */
const ROOT = process.env.GUARD_ROOT ?? fileURLToPath(new URL('..', import.meta.url))
const source = (file: string) => readFileSync(join(ROOT, 'src', file), 'utf8')
const reachable = reachableModules(ROOT, macRoots(ROOT))
const naming = (pattern: RegExp) => reachable.filter((file) => pattern.test(source(file))).sort()

describe('no macOS-reachable surface installs an update in place', () => {
  test('the census read the modules a Mac can mount', () => {
    for (const file of ['MacApp.tsx', 'MacSettings.tsx', 'UpdateBanner.tsx', 'App.tsx']) expect(reachable, file).toContain(file)
    expect(reachable.length).toBeGreaterThanOrEqual(40)
  })

  test('only UpdateBanner calls install_update, and only desktopApi can reach install_channel_downgrade', () => {
    expect(naming(/['"`]install_update['"`]/)).toEqual(['UpdateBanner.tsx'])
    expect(naming(/install_channel_downgrade|installDesktopChannelDowngrade/)).toEqual(['desktopApi.ts'])
  })

  test('"Restart to update" is said in one reachable module, the banner\'s non-Mac branch', () => {
    expect(naming(/Restart to update/).filter((file) => file !== 'macUpdateCopy.ts')).toEqual(['UpdateBanner.tsx'])
  })

  test('the banner only installs behind canInstallUpdate, which a Mac never passes', () => {
    const banner = source('UpdateBanner.tsx')
    expect(banner).toMatch(/const handleInstall = useCallback\(async \(\) => \{\s*if \(!installSupported\) return/)
    expect(banner).toContain('canInstallUpdate(caps)')
    expect(source('capabilities.tsx')).toMatch(/export function canInstallUpdate[^}]*host_os !== 'macos'/s)
  })

  test('the Settings sections never mention an install: the About row downloads', () => {
    for (const file of ['MacSettings.tsx', 'MacSecurity.tsx', 'MacSettingsExtras.tsx', 'macSettingsModel.ts', 'MacApp.tsx']) {
      expect(source(file), file).not.toMatch(/install_update|installDesktopChannelDowngrade|Restart to update/)
    }
  })
})
