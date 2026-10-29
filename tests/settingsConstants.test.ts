/**
 * One definition of the links and the channel list that the Mac Settings pane and the Windows views both show (PR #140
 * review, P3). A literal copied into a view is a link that opens the wrong page the day the first copy changes.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { ACCOUNT_SETTINGS_URL, RELEASE_CHANNELS, SECURITY_SETTINGS_URL } from '../src/settingsConstants'
import * as macSecurityModel from '../src/macSecurityModel'
import * as macSettingsModel from '../src/macSettingsModel'

const src = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')

describe('settingsConstants', () => {
  test('the web pages the Mac pane and the Windows views send a person to', () => {
    expect(SECURITY_SETTINGS_URL).toBe('https://app.beebeeb.io/settings/security')
    expect(ACCOUNT_SETTINGS_URL).toBe('https://app.beebeeb.io/settings/account')
  })

  test('the three channels, in the order the pickers show them', () => {
    expect(RELEASE_CHANNELS.map((channel) => channel.value)).toEqual(['stable', 'beta', 'alpha'])
    expect(RELEASE_CHANNELS.map((channel) => channel.label)).toEqual(['Stable', 'Beta', 'Alpha'])
  })

  test('the Mac models hand out the same objects, not copies', () => {
    expect(macSecurityModel.SECURITY_SETTINGS_URL).toBe(SECURITY_SETTINGS_URL)
    expect(macSettingsModel.ACCOUNT_SETTINGS_URL).toBe(ACCOUNT_SETTINGS_URL)
    expect(macSettingsModel.RELEASE_CHANNELS).toBe(RELEASE_CHANNELS)
  })

  test('no other file under src/ spells the two addresses or the channel hints out', () => {
    const files = [
      'macSecurityModel.ts', 'macSettingsModel.ts', 'MacSettings.tsx', 'MacSecurity.tsx',
      'windows/views/SecurityView.tsx', 'windows/views/AccountView.tsx', 'windows/views/SettingsView.tsx',
    ]
    for (const file of files) {
      const text = src(file)
      expect(text, `${file} spells out the security address`).not.toContain('app.beebeeb.io/settings/security')
      expect(text, `${file} spells out the account address`).not.toContain('app.beebeeb.io/settings/account')
      expect(text, `${file} lists the channels again`).not.toContain('Regular releases after validation.')
    }
    expect(src('settingsConstants.ts').match(/app\.beebeeb\.io\/settings\//g)).toHaveLength(2)
  })
})
