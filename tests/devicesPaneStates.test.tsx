// Devices and sessions must not look the same when the session list failed and when it succeeded with no rows.
// The three states are built by the pure helper in account/data.ts (what useAccount records from the commands)
// and drawn with renderToStaticMarkup — no fixture devices.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { commandUnavailableLabel, type AccountSession } from '../src/desktopApi'
import * as data from '../src/v2/sections/settings/account/data'
import * as pane from '../src/v2/sections/settings/account/DevicesPane'
import { elementsOf, textOf } from './fixtures/componentHarness'

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()

type Input = {
  sessions: { ok: true; value: { sessions: AccountSession[] } } | { ok: false; reason: string; unsupported: boolean } | null
  sync: { ok: true; value: { logged_in: boolean } } | { ok: false; reason: string; unsupported: boolean } | null
  version: { ok: true; value: unknown } | { ok: false; reason: string; unsupported: boolean } | null
  platform: { ok: true; value: string } | { ok: false; reason: string; unsupported: boolean } | null
}

function draw(input: Input, onRetry: () => void = () => {}) {
  const modelOf = (data as { devicesPaneModel?: (input: Input) => { kind: string } }).devicesPaneModel
  const List = (pane as { DevicesList?: (props: { model: unknown; onRetry: () => void }) => ReactElement }).DevicesList
  if (typeof modelOf !== 'function') throw new Error('devicesPaneModel is not exported from data.ts')
  if (typeof List !== 'function') throw new Error('DevicesList is not exported from DevicesPane.tsx')
  const model = modelOf(input)
  const tree = List({ model, onRetry })
  const html = renderToStaticMarkup(tree)
  return { model, tree, html, text: text(html) }
}

function session(over: Partial<AccountSession> & Pick<AccountSession, 'id' | 'device_name' | 'is_current'>): AccountSession {
  return {
    device_kind: 'linux',
    created_at: '2026-01-01T00:00:00Z',
    last_active_at: '2026-06-01T12:00:00Z',
    country_code: null,
    ...over,
  }
}

describe('Devices and sessions is never blank', () => {
  test('a failed list shows the error line and one Try again, not a device', () => {
    let retried = 0
    const unreachable = draw({
      sessions: { ok: false, reason: 'connection refused', unsupported: false },
      sync: { ok: true, value: { logged_in: true } },
      version: { ok: true, value: { version: '0.9.4', release: true } },
      platform: { ok: true, value: 'linux' },
    }, () => { retried += 1 })
    expect(unreachable.text).toContain(data.ERRORS.network)
    expect(unreachable.text).not.toContain('This PC')
    expect(unreachable.text).not.toContain('0.9.4')
    expect(unreachable.html).not.toContain('device-row')
    expect(unreachable.html.match(/Try again/g)).toHaveLength(1)
    const retry = elementsOf(unreachable.tree).filter((n) => n.props.onClick && textOf(n).trim() === 'Try again')
    expect(retry).toHaveLength(1)
    retry[0]!.props.onClick()
    expect(retried).toBe(1)

    const refused = draw({
      sessions: { ok: false, reason: 'account_session_list: not implemented in a browser', unsupported: true },
      sync: null,
      version: null,
      platform: null,
    })
    expect(refused.text).toContain(commandUnavailableLabel('account_session_list'))
    expect(refused.text).not.toContain(data.ERRORS.network)
    expect(refused.html.match(/Try again/g)).toHaveLength(1)
    expect(refused.html).not.toContain('device-row')

    const paneSrc = readFileSync(new URL('../src/v2/sections/settings/account/DevicesPane.tsx', import.meta.url), 'utf8')
    expect(paneSrc).toContain('onRetry={() => void reload()}')
  })

  test('an empty list while signed in shows this PC from local status, not a made-up session', () => {
    const view = draw({
      sessions: { ok: true, value: { sessions: [] } },
      sync: { ok: true, value: { logged_in: true } },
      version: { ok: true, value: { version: '0.9.4', release: true } },
      platform: { ok: true, value: 'linux' },
    })
    expect(view.model.kind).toBe('empty')
    const empty = view.model as { kind: 'empty'; thisPc: { name: string; session_ids: string[]; is_current: boolean; os: string; apps: string[] } | null }
    expect(empty.thisPc?.name).toBe('This PC')
    expect(empty.thisPc?.session_ids).toEqual([])
    expect(empty.thisPc?.is_current).toBe(true)
    expect(empty.thisPc?.os).toBe('Linux')
    expect(empty.thisPc?.apps).toContain('0.9.4')
    expect(view.text).toContain('This PC')
    expect(view.text).toContain('This device')
    expect(view.text).toContain('Linux')
    expect(view.text).toContain('0.9.4')
    expect(view.text).toContain('Other devices appear here after they sign in on this account.')
    expect(view.html.match(/data-testid="device-row"/g)).toHaveLength(1)
    expect(view.html).not.toContain('Try again')
    expect(view.text).not.toContain('Anna')

    const signedOut = draw({
      sessions: { ok: true, value: { sessions: [] } },
      sync: { ok: true, value: { logged_in: false } },
      version: { ok: true, value: { version: '0.9.4', release: false } },
      platform: { ok: true, value: 'linux' },
    })
    expect(signedOut.html).not.toContain('device-row')
    expect(signedOut.text).not.toContain('This PC')
    expect(signedOut.text).toContain('Other devices appear here after they sign in on this account.')

    const src = readFileSync(new URL('../src/v2/sections/settings/account/data.ts', import.meta.url), 'utf8')
    expect(src).toContain("'sync_status'")
    expect(src).toContain("'app_version'")
    expect(src).toContain("'desktop_platform'")
  })

  test('two rows keep the current device first', () => {
    const view = draw({
      sessions: {
        ok: true,
        value: {
          sessions: [
            session({ id: 'other', device_name: 'Kitchen laptop', device_kind: 'windows', is_current: false, last_active_at: '2026-08-01T00:00:00Z' }),
            session({ id: 'here', device_name: 'Desk', device_kind: 'linux', is_current: true, last_active_at: '2026-01-01T00:00:00Z' }),
          ],
        },
      },
      sync: { ok: true, value: { logged_in: true } },
      version: { ok: true, value: { version: '0.9.4', release: true } },
      platform: { ok: true, value: 'linux' },
    })
    expect(view.model.kind).toBe('rows')
    const rows = view.model as { kind: 'rows'; devices: { name: string; is_current: boolean }[] }
    expect(rows.devices.map((d) => d.name)).toEqual(['Desk', 'Kitchen laptop'])
    expect(rows.devices[0]?.is_current).toBe(true)
    expect(view.html.match(/data-testid="device-row"/g)).toHaveLength(2)
    expect(view.html.indexOf('Desk')).toBeGreaterThan(-1)
    expect(view.html.indexOf('Desk')).toBeLessThan(view.html.indexOf('Kitchen laptop'))
    expect(view.text).toContain('This device')
    expect(view.text).not.toContain('This PC')
    expect(view.text).not.toContain('Other devices appear here')
  })
})
