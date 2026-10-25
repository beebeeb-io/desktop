/**
 * The update banner on a Mac (task 2176, lead decision 2026-10-24 22:47; two-surface spec §0 ruling 5): the App Sandbox
 * denies the in-place install, so on macOS the banner says "Version X is available." and offers "Download update", which
 * opens the newest Mac disk image in the browser. It never offers a restart and it never calls `install_update`, whatever
 * the capabilities say. Windows and Linux are unchanged. The real `UpdateBanner` of `src/UpdateBanner.tsx`, run with the
 * component harness against a scripted backend and a scripted `update-available` event.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { canInstallUpdate, type DesktopCapabilities } from '../src/capabilities'
import * as desktopApi from '../src/desktopApi'
import * as macUpdateCopy from '../src/macUpdateCopy'
import * as macUpdateOffer from '../src/macUpdateOffer'
import { T } from '../src/windows/ui'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'
import fixtures from './fixtures/desktop-capabilities.json'

const React = { createElement, Fragment }
const caps = fixtures as unknown as Record<string, DesktopCapabilities>

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

function bus() {
  const listeners = new Map<string, (event: { payload: unknown }) => void>()
  return {
    listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
      listeners.set(name, callback)
      return () => { listeners.delete(name) }
    },
    emit: (name: string, payload: unknown) => listeners.get(name)?.({ payload }),
  }
}

async function openBanner(host: DesktopCapabilities | null, payload: Record<string, unknown> = { version: '0.8.13', body: 'Fixes the sync stall.', channel: 'stable', release_notes_url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.13' }) {
  const events = bus()
  const m = mount('UpdateBanner.tsx', 'UpdateBanner', {
    backend: { install_update: () => undefined, 'plugin:opener|open_url': () => undefined },
    expand: true,
    bindings: {
      ...desktopApi,
      ...macUpdateCopy,
      ...macUpdateOffer,
      ManualUpdateFeedback: () => null,
      useCapabilities: () => host,
      canInstallUpdate,
      listen: events.listen,
      T,
    },
  })
  mounted.push(m)
  await m.flush()
  events.emit('update-available', payload)
  await m.flush()
  await m.flush()
  const toast = m.toasts.at(-1)
  if (!toast) throw new Error('no banner was shown')
  return { m, toast }
}

const calls = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name)

describe('a Mac downloads its update', () => {
  test('the banner says the version and what to do, and offers "Download update", never a restart', async () => {
    const { toast } = await openBanner(caps.macos)
    expect(toast).toMatchObject({ id: 'desktop-update', variant: 'info', title: 'Version 0.8.13 is available.' })
    expect(textOf(toast.message)).toContain('Drag Beebeeb into Applications to replace this version.')
    expect(textOf(toast.message)).toContain('Release notes')
    expect(textOf(toast.message)).not.toMatch(/restart|installing|package manager/i)
    expect(toast.action.label).toBe('Download update')
    expect(toast.action.disabled).toBeUndefined()
  })

  test('the action opens the newest Mac disk image in the browser, and nothing is installed', async () => {
    const { m, toast } = await openBanner(caps.macos)
    await toast.action.onClick()
    await toast.action.onClick()
    expect(calls(m, 'plugin:opener|open_url').map((c) => c.args)).toEqual([{ url: macUpdateCopy.MACOS_DOWNLOAD_URL }, { url: macUpdateCopy.MACOS_DOWNLOAD_URL }])
    expect(calls(m, 'install_update')).toEqual([])
  })

  test('a Mac that is told it can install (a stale payload) still downloads: the host decides, not the install format', async () => {
    const { m, toast } = await openBanner({ ...caps.macos, update_format: 'app_bundle' })
    expect(toast.action.label).toBe('Download update')
    await toast.action.onClick()
    expect(calls(m, 'install_update')).toEqual([])
    expect(calls(m, 'plugin:opener|open_url')).toHaveLength(1)
  })

  test('the release notes link is the release the manifest named', async () => {
    const { m, toast } = await openBanner(caps.macos)
    const link = (function find(node: any): any {
      if (Array.isArray(node)) return node.map(find).find(Boolean)
      if (node?.type === 'button' && textOf(node.props.children) === 'Release notes') return node
      return node?.props ? find(node.props.children) : undefined
    })(toast.message)
    await link.props.onClick()
    expect(calls(m, 'plugin:opener|open_url')[0].args).toEqual({ url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.13' })
  })
})

describe('the offer follows the channel (review P2-4)', () => {
  test('a beta or alpha banner links to that release and names the channel; a payload with no channel is not taken for stable', async () => {
    for (const [channel, hint] of [['beta', 'This is a Beta release.'], ['alpha', 'This is an Alpha release.'], [undefined, 'This is a']] as const) {
      const { m, toast } = await openBanner(caps.macos, { version: '0.9.0-x.1', body: '', ...(channel ? { channel } : {}) })
      expect(textOf(toast.message)).toContain(hint)
      expect(toast.action.label).toBe('View release')
      await toast.action.onClick()
      expect(calls(m, 'plugin:opener|open_url').at(-1)?.args).toEqual({ url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.9.0-x.1' })
      expect(calls(m, 'install_update')).toEqual([])
    }
  })

  test('a stable banner keeps the newest disk image', async () => {
    const { m, toast } = await openBanner(caps.macos, { version: '0.8.13', body: '', channel: 'stable' })
    await toast.action.onClick()
    expect(calls(m, 'plugin:opener|open_url').at(-1)?.args).toEqual({ url: macUpdateCopy.MACOS_DOWNLOAD_URL })
  })
})

describe('Windows and Linux are unchanged', () => {
  test('an installable build still restarts to update, through install_update', async () => {
    for (const name of ['windows-nsis', 'windows-msi', 'linux-appimage']) {
      const { m, toast } = await openBanner(caps[name])
      expect(toast.title).toBe('Version 0.8.13 is available.')
      expect(textOf(toast.message)).toContain('Restart to apply the update.')
      expect(toast.action.label).toBe('Restart to update')
      await toast.action.onClick()
      expect(calls(m, 'install_update'), name).toHaveLength(1)
    }
  })

  test('a package-manager install says so and links to the downloads page', async () => {
    const { m, toast } = await openBanner(caps['linux-deb'])
    expect(textOf(toast.message)).toContain('Update with your package manager.')
    expect(toast.action.label).toBe('Downloads')
    await toast.action.onClick()
    expect(calls(m, 'plugin:opener|open_url')[0].args).toEqual({ url: 'https://beebeeb.io/download' })
    expect(calls(m, 'install_update')).toEqual([])
  })
})

// PR #140 delta review, P2-A: `App.tsx` re-renders the banner every 5 s (its status poll). The toast is shown again, with
// the same id, whenever the effect that shows it runs, and a person who dismissed it must not see it come back.
describe('a dismissed toast stays dismissed: the banner shows it once per update, not once per render', () => {
  const shownCount = (m: Mounted) => m.toasts.filter((t) => t.id === 'desktop-update').length

  test('on a Mac, however often the banner renders again, the toast is shown once', async () => {
    const { m } = await openBanner(caps.macos)
    const afterTheUpdate = shownCount(m)
    for (let tick = 0; tick < 6; tick++) {
      m.render()
      await m.flush()
    }
    expect(afterTheUpdate).toBe(1)
    expect(shownCount(m)).toBe(1)
  })

  test('on a beta update, whose offer is a link to the release, the same', async () => {
    const { m } = await openBanner(caps.macos, { version: '0.9.0', body: '', channel: 'beta' })
    for (let tick = 0; tick < 6; tick++) {
      m.render()
      await m.flush()
    }
    expect(shownCount(m)).toBe(1)
  })

  test('on Windows and Linux, the same', async () => {
    for (const name of ['windows-nsis', 'linux-appimage', 'linux-deb']) {
      const { m } = await openBanner(caps[name])
      for (let tick = 0; tick < 6; tick++) {
        m.render()
        await m.flush()
      }
      expect(shownCount(m), name).toBe(1)
    }
  })

  test('a newer update is a new toast: it is shown again for the new version', async () => {
    const events = bus()
    const m = mount('UpdateBanner.tsx', 'UpdateBanner', {
      backend: { install_update: () => undefined, 'plugin:opener|open_url': () => undefined },
      expand: true,
      bindings: { ...desktopApi, ...macUpdateCopy, ...macUpdateOffer, ManualUpdateFeedback: () => null, useCapabilities: () => caps.macos, canInstallUpdate, listen: events.listen, T },
    })
    mounted.push(m)
    await m.flush()
    events.emit('update-available', { version: '0.8.13', body: '', channel: 'stable' })
    await m.flush()
    await m.flush()
    events.emit('update-available', { version: '0.8.14', body: '', channel: 'stable' })
    await m.flush()
    await m.flush()
    expect(m.toasts.filter((t) => t.id === 'desktop-update').map((t) => t.title)).toEqual(['Version 0.8.13 is available.', 'Version 0.8.14 is available.'])
  })
})
