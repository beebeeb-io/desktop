// The settings chrome the DW-0701 frame draws (captured 2026-11-10 at 1280x800): the caption names Drive,
// the sidebar keeps Back to Drive and both groups, and Escape leaves the section when no dialog is open.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { allElements, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

async function show(node: import('react').ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
  return {
    container: container as MiniElement,
    text: () => dom.document.body.textContent ?? '',
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

// The frame's window draws the live banner, which reads the DriveSource. The stub answers nothing; the caption
// and the sidebar are what this test reads.
const source = {
  kind: 'desktop',
  storage: async () => null,
  sync: async () => null,
  list: async () => ({ items: [], asOf: null }),
  subscribe: () => () => {},
}

test('the DW-0701 frame caption names Drive', async () => {
  clearBody()
  const { FRAMES } = await import('../src/v2/frames/b12')
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const frame = FRAMES['DW-0701']
  if (!frame) throw new Error('DW-0701 is not registered')
  const view = await show(React.createElement(DriveSourceProvider, { source: source as never }, frame()))
  expect(view.text()).toContain('Settings - Beebeeb Drive')
  await view.unmount()
})

test('SettingsView keeps Back to Drive and both groups', async () => {
  clearBody()
  const { SettingsView } = await import('../src/v2/sections/settings/shell/SettingsView')
  const view = await show(React.createElement(SettingsView, { pane: 'general', override: React.createElement('div') }))
  const text = view.text()
  expect(text).toContain('Back to Drive')
  expect(text).toContain('esc')
  for (const label of ['General', 'Account', 'Security', 'Devices and sessions', 'Notifications']) {
    expect(text).toContain(label)
  }
  for (const label of ['Storage', 'Backups', 'Sync and File Explorer', 'Thumbnails and previews', 'Bandwidth']) {
    expect(text).toContain(label)
  }
  const nav = allElements(view.container).find((el) => el.getAttribute('data-testid') === 'settings-nav')
  const drive = allElements(view.container).find((el) => el.getAttribute('data-testid') === 'drive-nav')
  expect(nav?.textContent).toContain('General')
  expect(drive?.textContent).toContain('Storage')
  await view.unmount()
})

test('Escape calls onBack once when no dialog is open', async () => {
  clearBody()
  const { SettingsView } = await import('../src/v2/sections/settings/shell/SettingsView')
  let backs = 0
  const view = await show(React.createElement(SettingsView, {
    pane: 'general',
    override: React.createElement('div'),
    onBack: () => { backs += 1 },
  }))
  const event = {
    type: 'keydown',
    key: 'Escape',
    target: dom.document.body,
    currentTarget: dom.document,
    bubbles: true,
    cancelable: true,
    defaultPrevented: false,
    isTrusted: true,
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of dom.document.listenersFor('keydown', false)) fn(event)
  expect(backs).toBe(1)
  await view.unmount()
})
