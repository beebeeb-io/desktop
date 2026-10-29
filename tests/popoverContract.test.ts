import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import {
  POPOVER_PHASES,
  nextReviewTarget,
  parseEngineStatusEvent,
  parsePopoverSnapshot,
  parseReviewOpen,
  type ReviewTarget,
} from '../src/popoverContract'

const fixture = (name: string): unknown => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8'))

/** The phase names in the Rust enum, in declaration order, as snake_case. */
function rustPhases(): string[] {
  const source = readFileSync(new URL('../src-tauri/src/surfaces/phase.rs', import.meta.url), 'utf8').replace(/\r\n/g, '\n')
  const start = source.indexOf('pub enum PopoverPhase {')
  expect(start).toBeGreaterThan(-1)
  const body = source.slice(start, source.indexOf('\n}', start))
  return body
    .split('\n')
    .slice(1)
    .map((line) => line.trim().replace(/,$/, ''))
    .filter((line) => /^[A-Z][A-Za-z]*$/.test(line))
    .map((name) => name.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase())
}

describe('the popover snapshot contract (task 1683 slice 2)', () => {
  test('the TypeScript phase list is the Rust enum, name for name and in the same order', () => {
    const rust = rustPhases()
    expect(rust.length).toBe(13)
    expect([...POPOVER_PHASES]).toEqual(rust)
  })

  test('the JSON the Rust tests compare their output to passes the validator (3 of 3)', () => {
    const names = ['popover-snapshot.synced.json', 'popover-snapshot.syncing.json', 'popover-snapshot.error.json']
    const parsed = names.map((n) => parsePopoverSnapshot(fixture(n)))
    expect(parsed.filter((p) => p !== null).length).toBe(3)
    expect(parsed[1]?.activity.map((a) => a.state)).toEqual(['active', 'active', 'failed', 'done'])
    expect(parsed[1]?.activity[0]?.done_bytes).toBe(62)
    expect(parsed[2]?.reason).toEqual({ code: 'timeout', detail: 'timeout after 30 s' })
  })

  test('the validator refuses each way a snapshot can be wrong (9 of 9)', () => {
    const good = fixture('popover-snapshot.syncing.json') as Record<string, unknown>
    const broken: [string, (s: Record<string, unknown>) => void][] = [
      ['unknown phase', (s) => { s.phase = 'syncing_soon' }],
      ['missing account', (s) => { delete s.account }],
      ['engine count as a string', (s) => { (s.engine as Record<string, unknown>).files_remaining = '14' }],
      ['unknown finder setup', (s) => { (s.finder as Record<string, unknown>).setup = 'installed' }],
      ['storage without stale', (s) => { delete (s.storage as Record<string, unknown>).stale }],
      ['a row with a bad direction', (s) => { ((s.activity as Record<string, unknown>[])[0] as Record<string, unknown>).direction = 'sideways' }],
      ['a row with a bad state', (s) => { ((s.activity as Record<string, unknown>[])[0] as Record<string, unknown>).state = 'queued' }],
      ['reason without a code', (s) => { s.reason = { detail: 'x' } }],
      ['a row without a path', (s) => { delete ((s.activity as Record<string, unknown>[])[0] as Record<string, unknown>).path }],
    ]
    let refused = 0
    for (const [label, mutate] of broken) {
      const copy = JSON.parse(JSON.stringify(good)) as Record<string, unknown>
      mutate(copy)
      if (parsePopoverSnapshot(copy) === null) refused += 1
      else throw new Error(`accepted a snapshot with: ${label}`)
    }
    expect(refused).toBe(9)
    expect(parsePopoverSnapshot(null)).toBeNull()
    expect(parsePopoverSnapshot('synced')).toBeNull()
    expect(parsePopoverSnapshot([])).toBeNull()
  })

  test('the engine-status payloads Rust emits pass the validator, and a wrong state does not', () => {
    const syncing = parseEngineStatusEvent(fixture('engine-status.syncing.json'))
    expect(syncing?.state).toBe('syncing')
    expect(syncing?.legacy_state).toBe('idle')
    expect(syncing?.files_remaining).toBe(14)
    const offline = parseEngineStatusEvent(fixture('engine-status.offline.json'))
    expect(offline?.state).toBe('offline')
    expect(offline?.reason).toEqual({ code: 'connect', detail: null })
    // A complete payload whose only fault is the state: isolates the state check.
    const base = fixture('engine-status.syncing.json') as Record<string, unknown>
    expect(parseEngineStatusEvent({ ...base, state: 'napping' })).toBeNull()
    expect(parseEngineStatusEvent({ ...base, files_remaining: '14' })).toBeNull()
    expect(parseEngineStatusEvent({ ...base, reason: { detail: null } })).toBeNull()
    // The pause toggle's bare payload is not the full contract and is refused, not guessed.
    expect(parseEngineStatusEvent({ state: 'paused' })).toBeNull()
  })
})

describe('review:open (task 1683 slice 2; spec section 6 rule 9)', () => {
  test('the event Rust sends parses, keys are camelCase', () => {
    expect(parseReviewOpen(fixture('review-open.conflict.json'))).toEqual({
      fileId: 'f-1',
      fileName: 'ledger.xlsx',
      isText: false,
      mode: 'conflict',
    })
  })

  test('a malformed event is refused: no file id, a wrong mode, a non-boolean text flag', () => {
    const base = { fileId: 'f', fileName: 'a', isText: true, mode: 'conflict' }
    expect(parseReviewOpen({ ...base, fileId: '' })).toBeNull()
    expect(parseReviewOpen({ ...base, fileId: '   ' })).toBeNull()
    expect(parseReviewOpen({ ...base, mode: 'history' })).toBeNull()
    expect(parseReviewOpen({ ...base, isText: 'yes' })).toBeNull()
    expect(parseReviewOpen(null)).toBeNull()
    expect(parseReviewOpen({ ...base, fileName: '' })?.fileName).toBe('Unknown file')
  })

  test('two events for two files are two mounts in one window; a repeat is not a third', () => {
    const a = { fileId: 'a', fileName: 'a.txt', isText: true, mode: 'conflict' }
    const b = { fileId: 'b', fileName: 'b.txt', isText: false, mode: 'conflict' }
    let shown: ReviewTarget | null = null
    const mounts: string[] = []
    for (const event of [a, b, b, a]) {
      const next = nextReviewTarget(shown, event)
      if (next !== null && next.key !== shown?.key) mounts.push(next.key)
      shown = next
    }
    expect(mounts).toEqual(['a:conflict', 'b:conflict', 'a:conflict'])
    // The window itself is one reducer value, never an array of windows.
    expect(Array.isArray(shown)).toBe(false)
  })

  test('the same file in the other mode remounts, and a malformed event keeps what is shown', () => {
    const conflict = nextReviewTarget(null, { fileId: 'a', fileName: 'a', isText: true, mode: 'conflict' })
    const versions = nextReviewTarget(conflict, { fileId: 'a', fileName: 'a', isText: true, mode: 'versions' })
    expect(versions?.key).toBe('a:versions')
    expect(nextReviewTarget(versions, { fileId: '', mode: 'versions' })).toBe(versions)
    expect(nextReviewTarget(null, 42)).toBeNull()
  })

  test('a corrected file name or text flag updates the target without remounting', () => {
    const first = nextReviewTarget(null, { fileId: 'a', fileName: 'Unknown file', isText: false, mode: 'conflict' })
    const second = nextReviewTarget(first, { fileId: 'a', fileName: 'a.txt', isText: true, mode: 'conflict' })
    expect(second?.key).toBe(first?.key)
    expect(second?.fileName).toBe('a.txt')
    // An identical repeat is the very same object: nothing re-renders.
    expect(nextReviewTarget(second, { fileId: 'a', fileName: 'a.txt', isText: true, mode: 'conflict' })).toBe(second)
  })
})
