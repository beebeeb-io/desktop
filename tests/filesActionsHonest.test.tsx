// Files says what a person can do. An action this version cannot run never uses a build word or an internal state,
// and a row never shows the engine's blocked label.
import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { DriveItem, Listing } from '@beebeeb/shared/drive-ui'
import { CapabilityContext, rowMenuActions, type DesktopCapabilities } from '../src/capabilities'
import { FilesPane, type PaneProps } from '../src/v2/sections/files/FilesPane'
import { actionFailure, DEFAULT_PREFS, unavailableWords } from '../src/v2/sections/files/model'

const BAD = /\b(build|unsupported|unavailable|command)\b/i

test('open', () => {
  const text = actionFailure('open it', 'unsupported').text
  expect(text).toBe(unavailableWords('open'))
  expect(text.toLowerCase()).toContain('open')
  expect(text).toContain('Open it from File Explorer')
  expect(text).not.toMatch(BAD)
  expect(actionFailure('open it', 'unsupported').retry).toBe(false)
  expect(actionFailure('open it', 'failed')).toEqual({ text: 'Couldn’t open it.', retry: true })
})

test('reveal', () => {
  const text = actionFailure('show it in File Explorer', 'unsupported').text
  expect(text).toBe(unavailableWords('reveal'))
  expect(text.toLowerCase()).toContain('show')
  expect(text).toContain('Try again')
  expect(text).not.toMatch(BAD)
  expect(actionFailure('show it in File Explorer', 'failed').retry).toBe(true)
})

test('keep offline', () => {
  const text = actionFailure('keep it on this PC', 'unsupported').text
  expect(text).toBe(unavailableWords('keep offline'))
  expect(text.toLowerCase()).toContain('keep')
  expect(text).toContain('Try again')
  expect(text).not.toMatch(BAD)
  expect(actionFailure('keep it on this PC', 'offline')).toEqual({ text: 'Couldn’t keep it on this PC. Check your connection.', retry: true })
})

const blocked: DriveItem = {
  id: 'f1',
  parentId: null,
  name: 'Unavailable file ab12cd34ef567890',
  isFolder: false,
  size: 2048,
  modifiedAt: '2026-11-01T10:00:00',
  shared: { kind: 'none' },
  local: 'downloaded',
  hasThumbnail: false,
}

const listing: Listing = {
  folder: { id: null, name: 'Files' },
  path: [{ id: null, name: 'Files' }],
  items: [blocked],
  asOf: '2026-11-01T10:00:00',
  stale: false,
}

const caps: DesktopCapabilities = {
  host_os: 'windows',
  filesystem_integration: 'cloud_files',
  on_demand: true,
  native_credentials: true,
  tray: 'available',
  sharing: 'recipient_read_only',
  install_format: 'nsis',
  update_format: 'nsis',
  sync_mode_selection: false,
  metered_sync_control: false,
  sync_overlay_control: false,
  hydrate_all_control: false,
  cache_limit_control: false,
  commands: { file_open: 'unavailable' },
}

function rowHtml(): string {
  const p: PaneProps = {
    listing,
    items: [blocked],
    prefs: { ...DEFAULT_PREFS, panel: false },
    selection: new Set(),
    now: new Date('2026-11-09T12:00:00'),
  }
  return renderToStaticMarkup(createElement(CapabilityContext.Provider, { value: caps }, createElement(FilesPane, { p })))
}

test('one row state', () => {
  const open = rowMenuActions(caps, false).find((entry) => entry.id === 'open')
  expect(open).toEqual({ id: 'open', label: 'Open', disabled: true, sub: 'Not in this version' })
  const html = rowHtml()
  expect(html.toLowerCase()).not.toContain('unavailable')
  expect(html).toContain('Not in this version')
  expect(html).toContain('ab12cd34ef567890')
  expect(html).toContain('2 KB')
  expect(html).toContain('1 Nov 2026')
  expect(html).not.toMatch(BAD)
})
