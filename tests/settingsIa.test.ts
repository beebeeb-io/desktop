import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { settingLabel } from '../src/windows/settingsNavigation'
const read = (path: string) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8')
const settings = read('windows/views/SettingsView.tsx')
const mac = read('MacSettings.tsx')

describe('settings describe the choice and its scope', () => {
  test('storage country and advanced pages have useful names', () => {
    expect(settingLabel('data-residency')).toBe('Storage location')
    expect(settingLabel('advanced')).toBe('Appearance & storage')
  })
  test('encryption is a fixed fact, not a switch that does nothing', () => {
    expect(settings.includes('onChange={() => undefined}')).toBe(false)
    expect(settings.includes('control = <Chip>Always on</Chip>')).toBe(true)
  })
  test('theme choices appear in one interactive list', () => {
    expect(settings.match(/themeOptions\.map\(/g)?.length).toBe(1)
  })
  test('notification groups distinguish account events and this device', () => {
    expect(settings.includes('>Account notifications</div>')).toBe(true)
    expect(settings.includes('>On this device</div>')).toBe(true)
    expect(mac.includes('title="On this device"')).toBe(true)
    expect(read('MacSettingsExtras.tsx').includes('title="Account notifications"')).toBe(true)
  })
})
