/**
 * Review and Trash while the vault is locked (task 1683 slice 4; spec §7.6: "Locked: one card, the popover's sentence
 * and 'Unlock vault'. Review and Trash show the same card."; hi-fi section 3, "Home, vault locked").
 * What this proves: the card's words, that the one button unlocks and asks the window to read again, and that a
 * refusal is one toast. What it does NOT prove: layout (tests/render-mac-app.mjs).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as parts from '../src/macSettingsParts'
import { mount, type Mounted } from './fixtures/componentHarness'
import { button, lines, press } from './fixtures/sheetHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

function open(backend: Record<string, (args: any) => unknown> = {}) {
  let refreshed = 0
  const m = mount('LockedPane.tsx', 'LockedPane', {
    expand: true,
    backend: { unlock_vault: () => null, ...backend } as any,
    props: { onRefresh: () => { refreshed += 1 } },
    bindings: { ...desktopApi, Btn: parts.Btn, SettingsIcon: parts.SettingsIcon },
  })
  mounted.push(m)
  return { m, refreshed: () => refreshed }
}

describe('the locked card', () => {
  test('says what Home says, in the popover\'s words, with one amber button', () => {
    const { m } = open()
    expect(lines(m)).toEqual([
      'Your vault is locked',
      'Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.',
      'Unlock vault',
    ])
    expect(button(m, 'Unlock vault').props.className).toContain('ms-btn--primary')
  })

  test('the button unlocks, and the window is asked to read the state again', async () => {
    const { m, refreshed } = open()
    await press(m, 'Unlock vault')
    expect(m.calls.filter((c) => c.name === 'unlock_vault')).toHaveLength(1)
    expect(refreshed()).toBe(1)
    expect(m.toasts).toEqual([])
  })

  test('while it unlocks the button says so and cannot be pressed twice', async () => {
    let release: () => void = () => {}
    const { m } = open({ unlock_vault: () => new Promise<null>((resolve) => { release = () => resolve(null) }) })
    await m.clickNoWait('Unlock vault')
    expect(button(m, 'Unlocking…').props.disabled).toBe(true)
    release()
    await m.flush()
  })

  test('a refusal is one error toast with the reason, and the window still reads again', async () => {
    const { m, refreshed } = open({ unlock_vault: () => { throw new Error('keychain denied') } })
    await press(m, 'Unlock vault')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t unlock the vault' })
    expect(String(m.toasts[0].message)).toContain('keychain denied')
    expect(refreshed()).toBe(1)
  })
})
