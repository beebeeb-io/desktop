/**
 * Task 17b, fix round 1 (lead ruling): "show this file in Finder" (`open_in_finder`) reaches a person
 * from two places, the Shared roots page and the quick-search overlay. On a Mac its failure is ONE
 * fixed sentence and never `result.reason`; Windows and Linux keep their title and their error text.
 *
 * The component declarations run for real (tests/fixtures/componentHarness.ts) against a scripted
 * backend that rejects the command. What this does not prove: a real Finder, or React scheduling.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as finderSetup from '../src/finderSetup'
import { FINDER_SHOW_FILE_FAILED } from '../src/finderSetupCopy'
import { mount, elementsOf, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const LEAK = 'io.beebeeb.bridge 3'
const root = { id: 'root-1', name: 'Reports', owner_email: 'sam@beebeeb.io', permission: 'write', kind: 'folder', finder_path: null }
const hit = { file_id: 'f-1', name: 'report.pdf', rel_path: 'Reports/report.pdf', status: 'synced', size_bytes: 12, modified_at: 0 }

const toastText = (t: any) => [t.title, t.message].filter((part) => part != null).map((part) => textOf(part)).join(' ')

async function openShared(platform: string, failWith: string, helper: unknown = finderSetup.finderShowFileFailedToast) {
  const m = mount('pages/Shared.tsx', 'Shared', {
    backend: { list_shared_roots: () => [root], open_in_finder: () => { throw new Error(failWith) } },
    bindings: { ...desktopApi, usePlatformName: () => platform, finderShowFileFailedToast: helper, permissionLabel: () => 'Editable', kindLabel: () => 'Folder' },
  })
  mounted.push(m)
  await m.flush(); await m.flush()
  await m.click('Open')
  return m
}

async function openSearch(platform: string, failWith: string, helper: unknown = finderSetup.finderShowFileFailedToast) {
  const m = mount('DesktopQuickSearch.tsx', 'DesktopQuickSearch', {
    props: { open: true, onOpen() {}, onClose() {} },
    backend: {
      desktop_search_files: () => ({ query: 'rep', index_state: 'ready', indexed_file_count: 1, results: [hit] }),
      open_in_finder: () => { throw new Error(failWith) },
    },
    bindings: {
      ...desktopApi,
      usePlatformName: () => platform,
      finderShowFileFailedToast: helper,
      SearchGlyph: 'glyph',
      statusLabel: (status: string) => status,
      formatModifiedAt: () => '',
      SEARCH_LIMIT: 12,
      SEARCH_DEBOUNCE_MS: 0,
      EMPTY_RESULTS: [],
    },
  })
  mounted.push(m)
  await m.flush()
  const input = elementsOf(m.tree()).find((el) => el.type === 'input')!
  input.props.onChange({ target: { value: 'rep' } })
  m.render()
  for (let i = 0; i < 4; i++) await m.flush()
  const option = elementsOf(m.tree()).find((el) => el.props.role === 'option')
  if (!option) throw new Error('the search produced no result row')
  await option.props.onClick()
  await m.flush()
  return m
}

const surfaces = [
  ['Shared roots', openShared, 'Couldn’t open in Finder'],
  ['quick search', openSearch, 'Couldn’t open in Finder'],
] as const

describe('a failed open_in_finder on a Mac is one fixed sentence (task 17b, fix round 1)', () => {
  for (const [name, open] of surfaces) {
    test(`${name}: each reason, including one the unsupported heuristic matches, gives exactly the sentence and the code appears nowhere`, async () => {
      for (const reason of [LEAK, `invoke failed: ${LEAK}`, 'open Finder: No such file or directory', 'sync root not configured']) {
        const m = await open('macos', reason)
        expect(m.toasts.map(toastText)).toEqual([FINDER_SHOW_FILE_FAILED])
        expect(m.toasts.map((t) => t.variant)).toEqual(['error'])
        const everything = JSON.stringify(m.toasts) + textOf(m.tree())
        for (const text of ['io.beebeeb', 'No such file', 'sync root not configured', 'not wired']) expect(everything).not.toContain(text)
      }
    })
  }
})

describe('a failed open_in_finder off a Mac is unchanged (task 17b, fix round 1)', () => {
  const stub = () => { throw new Error('the macOS helper was reached off a Mac') }
  for (const [name, open, title] of surfaces) {
    for (const platform of ['windows', 'linux']) {
      test(`${name} on ${platform}: its title and the error text, and the macOS helper is never reached`, async () => {
        const reason = 'open Explorer: access is denied'
        const m = await open(platform, reason, stub)
        expect(m.toasts.map((t) => ({ variant: t.variant, title: t.title, message: t.message }))).toEqual([{ variant: 'error', title, message: reason }])
      })
    }
  }
})
