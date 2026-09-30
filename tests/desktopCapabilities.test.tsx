import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { CapabilityContext, CapabilityGate, CapabilityProvider, CapabilityResolutionError, canInstallUpdate, supportsRoute, type DesktopCapabilities } from '../src/capabilities'
import fixtures from './fixtures/desktop-capabilities.json'
import SettingsView, { AdvancedPanel } from '../src/windows/views/SettingsView'
import { SyncModeStep } from '../src/WindowsFirstRun'
import { ToastProvider } from '../src/windows/ui'
import type { DesktopConfig, SyncStatus } from '../src/desktopApi'

const cases = fixtures as Record<string, DesktopCapabilities>
const status = { logged_in: true, engine: 'running', syncing: 0 } as SyncStatus
function settings(host: string) {
  return renderToStaticMarkup(<CapabilityContext.Provider value={cases[host]}><ToastProvider>
    <SettingsView status={status} onOpenSignIn={() => { throw new Error('unexpected sign-in') }} />
  </ToastProvider></CapabilityContext.Provider>)
}

test('real Windows settings omit four unsupported preference actions and the no-op encryption switch', () => {
  const html = settings('windows-nsis')
  for (const label of ['Sync on metered connections', 'Show sync overlays in File Explorer', 'Files on Demand (online-only by default)', 'Keep files encrypted end-to-end']) {
    expect(html).not.toContain(`aria-label="${label}"`)
  }
  expect(html).toContain('Always on')
  expect(html).toContain('Free up space')
  expect(html).toContain('Pause syncing from the app menu')
  expect(html).toMatch(/class="capability-notice"[^>]*>[\s\S]*?Files download when opened/)
})

test('Mac settings preserve their existing controls and Finder routing', () => {
  const html = settings('macos')
  expect(html).toContain('aria-label="Sync on metered connections"')
  expect(html).toContain('aria-label="Files on Demand (online-only by default)"')
  expect(supportsRoute(cases.macos, 'finder')).toBe(true)
  expect(supportsRoute(cases['windows-nsis'], 'finder')).toBe(false)
})

test('Windows onboarding offers implemented on-demand behavior, not persisted-only mode choices', () => {
  const html = renderToStaticMarkup(<CapabilityContext.Provider value={cases['windows-msi']}><ToastProvider>
    <SyncModeStep onDone={() => {}} />
  </ToastProvider></CapabilityContext.Provider>)
  expect(html).toContain('Files download when you open them')
  expect(html).toContain('Continue')
  expect(html).not.toContain('Pick what lives on this PC')
  expect(html).not.toContain('Recommended')
})

test('unsupported direct routes never mount a child and provide an honest web alternative', () => {
  let mounted = 0
  function Child() { mounted++; return <button>Native action</button> }
  for (const name of ['linux-deb', 'unknown']) {
    for (const route of ['files', 'selective-sync', 'finder', 'explorer-integration', 'onboarding']) {
      const html = renderToStaticMarkup(<CapabilityContext.Provider value={cases[name]}><CapabilityGate route={route}><Child /></CapabilityGate></CapabilityContext.Provider>)
      expect(html).toContain('Open web app')
      expect(html).not.toContain('Native action')
      expect(html).toContain('class="capability-notice"')
      expect(html).toContain('class="button amber"')
    }
  }
  expect(mounted).toBe(0)
})

test('unresolved host never mounts children, regardless of layout selection', () => {
  let mounted = 0
  function Child() { mounted++; return <p>unsafe</p> }
  const html = renderToStaticMarkup(<CapabilityProvider><Child /></CapabilityProvider>)
  expect(html).toContain('Checking device support')
  expect(mounted).toBe(0)
})

test('capability load failure keeps a styled inline explanation and two separate recovery actions', () => {
  const html = renderToStaticMarkup(<CapabilityResolutionError onRetry={() => {}} />)
  expect(html).toContain('role="alert"')
  expect(html).toContain('Device support could not be checked.</h2>')
  expect(html).toContain('color:var(--ink-3)')
  expect(html).toContain('font-size:12px')
  expect(html).toMatch(/<button class="button amber" type="button">Retry<\/button>/)
  expect(html).toMatch(/<button class="button" type="button">Open web app<\/button>/)
  expect(html).not.toMatch(/<p[^>]*>[^<]*<button/)
})

test('seven Rust wire fixtures distinguish install support and implemented preferences', () => {
  expect(Object.keys(cases)).toHaveLength(7)
  for (const [name, caps] of Object.entries(cases)) {
    expect(canInstallUpdate(caps)).toBe(['macos', 'windows-nsis', 'windows-msi', 'linux-appimage'].includes(name))
    expect(caps.metered_sync_control).toBe(false)
    expect(caps.sync_overlay_control).toBe(false)
    expect(caps.sync_mode_selection).toBe(false)
    expect(caps.hydrate_all_control).toBe(false)
  }
  expect(canInstallUpdate(null)).toBe(false)
  expect(canInstallUpdate({ ...cases['windows-nsis'], install_format: 'unknown', update_format: 'manual' })).toBe(false)
})

test('capability IPC is registered in the shipping Rust handler', () => {
  const rust = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8').replace(/\r\n/g, '\n')
  expect(rust.slice(rust.indexOf('.invoke_handler(tauri::generate_handler!['))).toMatch(/\bdesktop_capabilities,/)
})


test('Windows hides the database-only disk cap while Mac retains its selector', () => {
  for (const host of ['windows-nsis', 'macos']) {
    const html = renderToStaticMarkup(<CapabilityContext.Provider value={cases[host]}><ToastProvider>
      <AdvancedPanel storage={null} config={{ theme: 'system', local_cache_limit_bytes: 0 } as DesktopConfig} onConfigChange={() => {}} />
    </ToastProvider></CapabilityContext.Provider>)
    expect(html.includes('aria-label="Local cache limit"')).toBe(host === 'macos')
    expect(html).toContain('aria-label="Theme"')
    if (host === 'windows-nsis') {
      expect(html).toContain('Use Free up space')
      expect(html).toContain('class="capability-notice"')
    }
  }
})
