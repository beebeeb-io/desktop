/** Product copy uses the EU; only transparency surfaces name the current city. */
import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const root = join(import.meta.dir, '..')
const read = (path: string) => readFileSync(join(root, path), 'utf8')

const productSurfaces = [
  'src/Onboarding.tsx',
  'src/WindowsFirstRun.tsx',
  'src/WindowsApp.tsx',
  'src/windows/KnownFolderOnboarding.tsx',
  'src/windows/views/ActivityView.tsx',
  'src/windows/views/InsightsView.tsx',
  'src/windows/views/SelectiveSyncView.tsx',
  'src/windows/views/AccountView.tsx',
  'src/pages/Account.tsx',
]

describe('product residency copy', () => {
  for (const file of productSurfaces) {
    test(`${file} uses EU copy without a city lookup`, () => {
      const source = read(file)
      expect(source.includes('Stored in the EU')).toBe(true)
      expect(source.match(/Falkenstein|Germany|useRegion|regionCityFromCode|EU servers/g)).toBeNull()
    })
  }
  test('Explorer and Finder integration introduction uses EU copy', () => {
    const source = read('src/windows/views/SettingsView.tsx')
    expect(source.includes('before they leave for the EU.')).toBe(true)
    expect(source.match(/useRegionCity|\$\{regionCity\}/g)).toBeNull()
  })
})

test('frontend, legacy Windows and native tray/menu sources never name a provider', () => {
  function sources(directory: string): string[] {
    return readdirSync(join(root, directory), { withFileTypes: true }).flatMap(entry => {
      const path = join(directory, entry.name)
      return entry.isDirectory() ? sources(path) : /\.(tsx?|rs)$/.test(entry.name) ? [path] : []
    })
  }
  const files = [...sources('src'), ...sources('windows/src'), 'src-tauri/src/lib.rs']
  expect(files.length).toBeGreaterThan(50)
  for (const file of files) expect(read(file).match(/hetzner|ovhcloud|digitalocean/gi)).toBeNull()
})
