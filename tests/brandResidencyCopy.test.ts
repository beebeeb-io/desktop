/** Product copy follows the effective continent; transparency may name the city. */
import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const root = join(import.meta.dir, '..')
// CRLF-tolerant: Windows checkouts (autocrlf) must scan identically to LF (task 1639).
const read = (path: string) => readFileSync(join(root, path), 'utf8').replace(/\r\n/g, '\n')

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

describe('product residency copy wiring', () => {
  for (const file of [...productSurfaces, 'src/windows/views/SettingsView.tsx']) {
    test(`${file} derives every product location claim from the shared hook`, () => {
      const source = read(file)
      const product = file.endsWith('SettingsView.tsx')
        ? source.slice(source.indexOf('function ExplorerIntegrationPanel'), source.indexOf('export function UpdatesPanel'))
        : source
      expect(/useRegionLabel\((?:step|currentStep)?\)/.test(product)).toBe(true)
      expect(product.includes('{regionLabel}')).toBe(true)
      expect(product.match(/Stored in the EU|before they leave for the EU|Falkenstein|Germany|regionCityFromCode|useRegionCity|EU servers/g)).toBeNull()
    })
  }
  for (const file of ['DevicesView', 'SecurityView']) {
    test(`${file} retains residency-neutral copy`, () => {
      const source = read(`src/windows/views/${file}.tsx`)
      expect(source.match(/Stored in|the EU|Falkenstein|Germany|regionCity|useRegion/g)).toBeNull()
    })
  }
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
