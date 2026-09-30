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

test('a delayed old-account region response cannot fill the new cache', async () => {
  const previous = globalThis.window
  let revision = 10
  let resolveOld!: (value: unknown) => void
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (name: string) => {
    if (name === 'sync_status') return { logged_in: true, session_revision: revision }
    if (name === 'account_region') return revision === 10
      ? new Promise(resolve => { resolveOld = resolve })
      : { preferred_region: 'us' }
    throw new Error(name)
  } } } as unknown as Window & typeof globalThis
  try {
    await loadSyncStatus()
    const old = fetchRegion()
    revision = 12
    await loadSyncStatus()
    expect((await fetchRegion())?.preferred_region).toBe('us')
    resolveOld({ preferred_region: 'europe' })
    expect(await old).toBeNull()
    expect((await fetchRegion())?.preferred_region).toBe('us')
  } finally { globalThis.window = previous }
})
