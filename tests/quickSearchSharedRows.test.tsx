/**
 * Task 1885, fix round 1 (I3): shared-with-me rows are never sent to Finder.
 *
 * Shared content is webapp-only (ruling 1701): it is not in Finder. Quick search finds it (the index holds every row),
 * but its only action is "show in Finder", so on a Mac those rows are not offered. Windows and Linux keep every row.
 *
 * The component declaration runs for real (tests/fixtures/componentHarness.ts) against a scripted backend. What this does
 * not prove: a real Finder, or React scheduling.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import { mount, elementsOf, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const mine = { file_id: 'f-mine', name: 'report-mine.pdf', rel_path: 'Reports/report-mine.pdf', status: 'synced', size_bytes: 12, modified_at: 0, shared_with_me: false }
const shared = { file_id: 'f-shared', name: 'report-theirs.pdf', rel_path: 'Shared/report-theirs.pdf', status: 'synced', size_bytes: 12, modified_at: 0, shared_with_me: true }

async function search(platform: string, results: unknown[]) {
  const m = mount('DesktopQuickSearch.tsx', 'DesktopQuickSearch', {
    props: { open: true, onOpen() {}, onClose() {} },
    backend: {
      desktop_search_files: () => ({ query: 'rep', index_state: 'ready', indexed_file_count: results.length, results }),
      open_in_finder: () => undefined,
    },
    bindings: {
      ...desktopApi,
      usePlatformName: () => platform,
      finderShowFileFailedToast: () => ({ variant: 'error', message: '' }),
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
  return m
}

const rows = (m: Mounted) => elementsOf(m.tree()).filter((el) => el.props.role === 'option')
const names = (m: Mounted) => rows(m).map((row) => textOf(row))

describe('quick search never offers a shared-with-me row for Finder on a Mac (task 1885, I3)', () => {
  test('macOS: the shared row is not offered, and clicking the other row still asks for Finder', async () => {
    const m = await search('macos', [shared, mine])
    expect(rows(m)).toHaveLength(1)
    expect(names(m)[0]).toContain('report-mine.pdf')
    expect(names(m).join(' ')).not.toContain('report-theirs.pdf')
    await rows(m)[0].props.onClick()
    await m.flush()
    expect(m.calls.filter((call) => call.name === 'open_in_finder').map((call) => call.args.itemId)).toEqual(['f-mine'])
  })

  test('macOS: a search that matches only shared rows says there is no match, and no row can ask for Finder', async () => {
    const m = await search('macos', [shared])
    expect(rows(m)).toHaveLength(0)
    expect(textOf(m.tree())).toContain('No matching files.')
    expect(m.calls.filter((call) => call.name === 'open_in_finder')).toEqual([])
  })

  for (const platform of ['windows', 'linux']) {
    test(`${platform}: every row is offered, as before`, async () => {
      const m = await search(platform, [shared, mine])
      expect(names(m).map((text) => (text.includes('report-theirs') ? 'shared' : 'mine'))).toEqual(['shared', 'mine'])
    })
  }

  test('quick search asks the backend for rows to show in Finder, so a Mac drops shared rows before the limit (n1)', async () => {
    const m = await search('macos', [mine])
    const asked = m.calls.filter((call) => call.name === 'desktop_search_files')
    expect(asked.length).toBeGreaterThan(0)
    for (const call of asked) expect(call.args).toMatchObject({ query: 'rep', limit: 12, forFinder: true })
  })

  test('the filter is a pure function of the platform and the rows', () => {
    expect(desktopApi.finderRevealableResults('macos', [shared, mine] as any).map((r) => r.file_id)).toEqual(['f-mine'])
    expect(desktopApi.finderRevealableResults('windows', [shared, mine] as any).map((r) => r.file_id)).toEqual(['f-shared', 'f-mine'])
    expect(desktopApi.finderRevealableResults('macos', [])).toEqual([])
  })
})
