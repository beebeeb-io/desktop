/**
 * A local build's Cargo version is not a release. General and About say so, and they do not
 * offer that build an update. A release build still shows its version and the offer.
 */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { allElements, installMiniDom, type MiniDomInstall } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

function stub(appVersion: () => Promise<unknown>) {
  let callbackId = 1
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return callbackId++ },
    invoke(cmd: string) {
      if (cmd === 'app_version') return appVersion()
      if (cmd === 'popover_snapshot') return Promise.reject(new Error('no snapshot'))
      if (cmd === 'get_desktop_config') return Promise.resolve({})
      if (cmd === 'consume_menu_update_check') return Promise.resolve(false)
      return Promise.resolve(1)
    },
  }
  dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
}

async function shown(appVersion: () => Promise<unknown>): Promise<{ text: string; banner: boolean }> {
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { General } = await import('../src/v2/sections/settings/shell/general')
  const { AboutToday } = await import('../src/v2/sections/settings/today/about')
  const { noteUpdate } = await import('../src/v2/sections/settings/shell/updates')
  noteUpdate('0.9.0')
  stub(appVersion)
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  const value = { pane: 'general' as const, go: () => {}, dialog: null, open: () => {}, close: () => {} }
  await React.act(async () => {
    root.render(React.createElement(Ctx.Provider, { value }, React.createElement(General), React.createElement(AboutToday)))
  })
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
  const text = dom.document.body.textContent
  const banner = allElements(dom.document.body).some((el) => el.getAttribute('data-testid') === 'update-banner')
  await React.act(async () => root.unmount())
  container.parentNode?.removeChild(container)
  noteUpdate(null)
  return { text, banner }
}

test.serial('a dev build names itself and does not offer an update', async () => {
  const { text, banner } = await shown(() => Promise.resolve('0.1.0'))
  expect(text).toContain('Development build (0.1.0)')
  expect(text).toContain('Updates are checked, but installing one replaces this build.')
  expect(text.toLowerCase()).not.toContain('update available')
  expect(banner).toBe(false)
})

test.serial('a release build shows its version and keeps the offer', async () => {
  const { text, banner } = await shown(() => Promise.resolve({ version: '0.8.24', release: true }))
  expect(text).toContain('Version 0.8.24')
  expect(text.toLowerCase()).toContain('update available')
  expect(banner).toBe(true)
  expect(text).not.toContain('Development build')
})

test.serial('no answer invents neither a development build nor an update', async () => {
  const { text, banner } = await shown(() => Promise.reject(new Error('app_version failed')))
  expect(text).not.toContain('Development build')
  expect(text.toLowerCase()).not.toContain('update available')
  expect(text).not.toMatch(/Version \d/)
  expect(banner).toBe(false)
})
