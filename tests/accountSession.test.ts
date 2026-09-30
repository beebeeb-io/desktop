import { expect, test } from 'bun:test'
import { fetchRegion, loadSyncStatus } from '../src/desktopApi'

test('region cache refetches after account switch between status polls', async () => {
  const previous = globalThis.window
  let revision = 1
  let calls = 0
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (name: string) => {
    if (name === 'sync_status') return { logged_in: true, session_revision: revision }
    if (name === 'account_region') { calls++; return { preferred_region: revision === 1 ? 'europe' : 'us' } }
    throw new Error(name)
  } } } as unknown as Window & typeof globalThis
  try {
    await loadSyncStatus()
    expect((await fetchRegion())?.preferred_region).toBe('europe')
    revision = 3
    await loadSyncStatus()
    expect((await fetchRegion())?.preferred_region).toBe('us')
    expect(calls).toBe(2)
  } finally { globalThis.window = previous }
})
