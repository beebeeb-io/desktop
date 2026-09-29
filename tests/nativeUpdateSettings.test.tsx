import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks'
import { UpdatesPanel } from '../src/windows/views/SettingsView'
import { CapabilityContext, type DesktopCapabilities } from '../src/capabilities'
import { DEFAULT_CONFIG } from '../src/desktopApi'
import fixtures from './fixtures/desktop-capabilities.json'
import { desktopUpdateCheck, connectNativeUpdateMenu } from '../src/windows/manualUpdateCheck'

// Real IPC wrapper, controller and Settings rendering; no browser/native claim.
test('native failure updates rendered Settings and preserves the debug manual-install alternative', async () => {
  const previousWindow = globalThis.window
  globalThis.window = { location: { search: '?platform=windows' } } as Window & typeof globalThis
  let commands = 0
  let settle!: () => void
  let onMenu = () => {}
  let pending = false
  const render = () => renderToStaticMarkup(<CapabilityContext.Provider value={fixtures.unknown as DesktopCapabilities}>
    <UpdatesPanel config={DEFAULT_CONFIG} onConfigChange={() => {}} />
  </CapabilityContext.Provider>)
  desktopUpdateCheck.reset()
  mockIPC(async (command) => {
    expect(command).toBe('check_for_updates_now')
    commands++
    await new Promise<void>((resolve) => { settle = resolve })
    throw 'Could not reach the stable update manifest: connection refused'
  })
  const disconnect = connectNativeUpdateMenu(async (callback) => {
    onMenu = callback
    return () => {}
  }, async () => { const value = pending; pending = false; return { ok: true, value } }, desktopUpdateCheck.check)
  try {
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(render()).toContain('Not checked')
    pending = true
    onMenu()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(commands).toBe(1)
    const checking = render()
    settle()
    await new Promise((resolve) => setTimeout(resolve, 0))
    const failed = render()
    expect(failed).not.toContain('Not checked')
    expect(checking).toContain('Checking...')
    expect(failed).toContain('Check failed')
    expect(failed).toContain('Could not reach the stable update manifest: connection refused')
    expect(failed).toContain('Try again')
    expect(failed).toContain('In-app installation is unavailable for this package')
    expect(failed).toContain('Downloads')
    expect(failed).not.toContain('Restart to update')
  } finally {
    disconnect()
    clearMocks()
    globalThis.window = previousWindow
    desktopUpdateCheck.reset()
  }
})
