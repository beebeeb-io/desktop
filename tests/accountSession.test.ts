import { afterEach, describe, expect, test } from 'bun:test'
import { clearSignOutWarning, heldSignOutWarning, holdSignOutWarning, observeSignOutWarning, subscribeSignOutWarning } from '../src/accountSession'
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

// FB-24 row 12: a sign-out's warning sentence outlives AccountSessionBoundary's remount, and goes on the next
// sign-in. These pin which status reads count as that sign-in (the remount itself is in
// signOutWarningSurvivesBoundary.test.tsx).
describe('the held sign-out sentence', () => {
  afterEach(() => clearSignOutWarning())
  const SENTENCE = 'You are signed out, but Beebeeb could not confirm that it was removed from Finder.'

  test('a read that began before the sign-out (signed in, old revision) clears nothing, even before a signed-out read', () => {
    holdSignOutWarning(SENTENCE)
    observeSignOutWarning(true, 4)
    expect(heldSignOutWarning()).toBe(SENTENCE)
    observeSignOutWarning(false, 6)
    observeSignOutWarning(true, 4)
    expect(heldSignOutWarning()).toBe(SENTENCE)
  })

  test('signed in again at a newer revision than the signed-out one is the next sign-in: the sentence goes', () => {
    holdSignOutWarning(SENTENCE)
    observeSignOutWarning(false, 5)
    observeSignOutWarning(false, 6)
    observeSignOutWarning(true, 6)
    expect(heldSignOutWarning()).toBe(SENTENCE)
    observeSignOutWarning(true, 7)
    expect(heldSignOutWarning()).toBeNull()
  })

  test('a sign-out with no warning holds nothing, and clearing tells the surfaces', () => {
    let told = 0
    const stop = subscribeSignOutWarning(() => { told += 1 })
    holdSignOutWarning(SENTENCE)
    holdSignOutWarning(null)
    expect(heldSignOutWarning()).toBeNull()
    holdSignOutWarning(SENTENCE)
    clearSignOutWarning()
    clearSignOutWarning()
    stop()
    expect(heldSignOutWarning()).toBeNull()
    expect(told).toBe(4)
  })

  test('every status read feeds it before the revision is observed, so a sign-in clears it before the remount', async () => {
    const previous = globalThis.window
    let status = { logged_in: false, session_revision: 40 }
    globalThis.window = { __TAURI_INTERNALS__: { invoke: async (name: string) => {
      if (name === 'sync_status') return status
      throw new Error(name)
    } } } as unknown as Window & typeof globalThis
    try {
      holdSignOutWarning(SENTENCE)
      await loadSyncStatus()
      let atRemount: string | null | undefined
      const { subscribeAccountSession } = await import('../src/accountSession')
      const stop = subscribeAccountSession(() => { atRemount = heldSignOutWarning() })
      status = { logged_in: true, session_revision: 42 }
      await loadSyncStatus()
      stop()
      expect(atRemount).toBeNull()
    } finally { globalThis.window = previous }
  })
})
