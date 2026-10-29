/**
 * Task 2191 (spec D11, Decision (lead, 2026-10-26)): the Windows main window's Disconnect asks
 * before files from before the last update are copied or deleted. The real DisconnectSection
 * runs with the component harness against a scripted backend.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import { UnknownFilesSignOutDialog } from '../src/windows/UnknownFilesSignOutDialog'
import { mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

function openDisconnect(unknown: { count: number; folder: string | null }) {
  const m = mount('windows/views/AccountView.tsx', 'DisconnectSection', {
    backend: { sign_out_unknown_files: () => unknown, clear_session: () => ({ warning: null }) },
    bindings: { ...desktopApi, T: new Proxy({}, { get: () => '' }), DANGER: 'red', UnknownFilesSignOutDialog },
  })
  mounted.push(m)
  return m
}

const dialog = (m: Mounted) => m.elements().find((el) => el.type === UnknownFilesSignOutDialog)
const signOuts = (m: Mounted) => m.calls.filter((call) => call.name === 'clear_session')

describe('Windows Disconnect with files from before the last update (task 2191)', () => {
  test('it asks first, and signs out only with the answer', async () => {
    const m = openDisconnect({ count: 2, folder: 'C:\\Users\\a\\Beebeeb kept files 2026-10-26 1200' })
    await m.flush()
    await m.click('Disconnect')
    await m.click('Disconnect')
    expect(dialog(m)?.props.count).toBe(2)
    expect(signOuts(m)).toEqual([])
    await dialog(m)!.props.onChoose('keep')
    await m.flush()
    expect(signOuts(m).map((call) => call.args)).toEqual([
      { unknownFiles: 'keep', keptFolder: 'C:\\Users\\a\\Beebeeb kept files 2026-10-26 1200' },
    ])
  })

  test('"Delete them" is passed on as such', async () => {
    const m = openDisconnect({ count: 1, folder: 'D:\\kept' })
    await m.flush()
    await m.click('Disconnect')
    await m.click('Disconnect')
    await dialog(m)!.props.onChoose('delete')
    await m.flush()
    expect(signOuts(m).map((call) => call.args)).toEqual([{ unknownFiles: 'delete', keptFolder: 'D:\\kept' }])
  })

  test('with none, it signs out as before, without asking', async () => {
    const m = openDisconnect({ count: 0, folder: null })
    await m.flush()
    await m.click('Disconnect')
    await m.click('Disconnect')
    expect(dialog(m)).toBeUndefined()
    expect(signOuts(m).length).toBe(1)
  })
})
