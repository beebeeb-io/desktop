/**
 * `sync_status.engine_refusal` (Lane R, ruling P; must-render rows 8–10): why sync did not start,
 * when the local-data binding (or an unconfirmed earlier engine stop) refused the engine start. It
 * is `{code, sentence}` with a closed set of codes, or null. The frontend parses it strictly: a value
 * outside the contract is no refusal (the surface keeps its own generic sentence) rather than a
 * guess, and `loadSyncStatus` hands every caller the parsed value.
 *
 * The codes and sentences are read from the Rust source (account_binding.rs), so this file pins the
 * contract on both sides instead of a copy of it.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { ENGINE_REFUSAL_CODES, loadEngineRefusal, loadSyncStatus, parseEngineRefusal } from '../src/desktopApi'
import { rustSource, rustStr } from './fixtures/rustConstants'

const SENTENCE = {
  identity_unknown: rustStr('account_binding.rs', 'IDENTITY_UNKNOWN'),
  other_account_on_windows: rustStr('account_binding.rs', 'OTHER_ACCOUNT_ON_WINDOWS'),
  engine_stop_unconfirmed: rustStr('account_binding.rs', 'ENGINE_STOP_UNCONFIRMED'),
}

describe('the engine_refusal contract', () => {
  test('the closed set of codes is exactly what Rust\'s Refusal::code() returns', () => {
    const source = rustSource('account_binding.rs')
    const body = source.slice(source.indexOf('pub fn code(self)'), source.indexOf('pub fn sentence(self)'))
    const rust = [...body.matchAll(/=> "([a-z_]+)"/g)].map((m) => m[1]).sort()
    expect(rust).toHaveLength(3)
    expect([...ENGINE_REFUSAL_CODES].sort()).toEqual(rust)
  })

  test('sync_status sends it under that name, as {code, sentence} or null', () => {
    const lib = rustSource('lib.rs')
    expect(lib).toContain('"engine_refusal": engine_refusal_view(&acct),')
    expect(lib).toContain('serde_json::json!({ "code": refusal.code(), "sentence": refusal.sentence() })')
  })
})

describe('parseEngineRefusal', () => {
  test('each code with its sentence parses', () => {
    for (const code of ENGINE_REFUSAL_CODES) {
      expect(parseEngineRefusal({ code, sentence: SENTENCE[code] })).toEqual({ code, sentence: SENTENCE[code] })
    }
  })

  test('null and a missing field are no refusal', () => {
    expect(parseEngineRefusal(null)).toBeNull()
    expect(parseEngineRefusal(undefined)).toBeNull()
  })

  test('anything outside the contract is rejected, never guessed', () => {
    for (const wrong of [
      { code: 'something_new', sentence: 'x' },
      { code: 'IDENTITY_UNKNOWN', sentence: 'x' },
      { code: 'identity_unknown' },
      { code: 'identity_unknown', sentence: '' },
      { code: 'identity_unknown', sentence: '   ' },
      { code: 'identity_unknown', sentence: 42 },
      { sentence: 'x' },
      'identity_unknown',
      ['identity_unknown', 'x'],
      42,
    ]) {
      expect(parseEngineRefusal(wrong)).toBeNull()
    }
  })
})

describe('loadSyncStatus and loadEngineRefusal hand over the parsed refusal', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })
  const backend = (engine_refusal: unknown) => {
    ;(globalThis as any).window = {
      __TAURI_INTERNALS__: {
        invoke: async (name: string) => {
          if (name !== 'sync_status') throw new Error(`unscripted ${name}`)
          return { logged_in: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal }
        },
      },
    }
  }

  test('a valid refusal reaches the caller', async () => {
    backend({ code: 'identity_unknown', sentence: SENTENCE.identity_unknown })
    expect((await loadSyncStatus())?.engine_refusal).toEqual({ code: 'identity_unknown', sentence: SENTENCE.identity_unknown })
    expect(await loadEngineRefusal()).toEqual({ code: 'identity_unknown', sentence: SENTENCE.identity_unknown })
  })

  test('a malformed one reaches the caller as null', async () => {
    backend({ code: 'identity_unknown', sentence: 7 })
    expect((await loadSyncStatus())?.engine_refusal).toBeNull()
    expect(await loadEngineRefusal()).toBeNull()
  })

  test('a status that cannot be read is no refusal', async () => {
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async () => { throw new Error('down') } } }
    expect(await loadEngineRefusal()).toBeNull()
  })
})
