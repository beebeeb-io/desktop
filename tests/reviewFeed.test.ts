/**
 * The Review feed (task 1683 slice 4; spec §7.6): one place reads `list_version_conflict_center` for the sidebar badge
 * and the Review pane, on the window's 5 s cadence and again the moment a conflict is found or resolved.
 * What this proves: the states a reader can be in, that a poll never churns the window when nothing changed, that a
 * failed poll never wipes what was already shown, and that stopping really stops. What it does NOT prove: that
 * Rust's feed is right (src-tauri tests) or the window around it (tests/macApp.test.tsx).
 */
import { describe, expect, test } from 'bun:test'
import { commandUnavailableLabel, type CommandResult, type VersionConflictEntry } from '../src/desktopApi'
import { connectReviewFeed, ReviewFeed, REVIEW_FEED_INTERVAL_MS } from '../src/reviewFeed'

const entry = (id: string, over: Partial<VersionConflictEntry> = {}): VersionConflictEntry => ({
  id, file_id: `f-${id}`, file_name: `${id}.md`, kind: 'conflict', status: 'needs review', updated_at: 1000, detail: 'd', action: 'open_conflict', ...over,
})

type Read = () => Promise<CommandResult<VersionConflictEntry[]>>
const ok = (entries: VersionConflictEntry[]): CommandResult<VersionConflictEntry[]> => ({ ok: true, value: entries })
const bad = (reason: string, unsupported = false): CommandResult<VersionConflictEntry[]> => ({ ok: false, reason, unsupported } as CommandResult<VersionConflictEntry[]>)

/** A scripted reader and a clock we turn by hand. */
function harness(script: Array<CommandResult<VersionConflictEntry[]> | Promise<CommandResult<VersionConflictEntry[]>>>) {
  const reads: number[] = []
  const read: Read = () => {
    reads.push(reads.length)
    const next = script[Math.min(reads.length - 1, script.length - 1)]
    return Promise.resolve(next)
  }
  const timers = new Map<number, () => void>()
  let nextId = 1
  const clock = {
    setInterval: (fn: () => void, ms: number) => { expect(ms).toBe(REVIEW_FEED_INTERVAL_MS); const id = nextId++; timers.set(id, fn); return id },
    clearInterval: (id: number) => { timers.delete(id) },
  }
  const feed = new ReviewFeed(read, clock)
  const seen: unknown[] = []
  feed.subscribe(() => seen.push(feed.state()))
  const tick = () => { for (const fn of [...timers.values()]) fn() }
  const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve() }
  return { feed, reads, timers, tick, settle, seen }
}

describe('the cadence is the window\'s own', () => {
  test('5 seconds, like the snapshot read', () => {
    expect(REVIEW_FEED_INTERVAL_MS).toBe(5000)
  })
})

describe('states', () => {
  test('before the first answer it is loading, with nothing in it', () => {
    const { feed } = harness([ok([])])
    expect(feed.state()).toEqual({ status: 'loading', entries: [], reason: null })
  })

  test('starting reads at once and then on the timer; the answer is ready, in the order Rust sent it', async () => {
    const { feed, reads, settle, tick } = harness([ok([entry('a'), entry('b')])])
    feed.start()
    expect(reads).toHaveLength(1)
    await settle()
    expect(feed.state().status).toBe('ready')
    expect(feed.state().entries.map((e) => e.id)).toEqual(['a', 'b'])
    tick()
    expect(reads).toHaveLength(2)
  })

  test('a first read that fails is "failed" with the reason; the next good read heals it', async () => {
    const { feed, settle, tick } = harness([bad('state.db is locked'), ok([entry('a')])])
    feed.start()
    await settle()
    expect(feed.state()).toEqual({ status: 'failed', entries: [], reason: 'state.db is locked' })
    tick()
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', reason: null })
    expect(feed.state().entries).toHaveLength(1)
  })

  test('a build without the command says so in the shared words, not "invoke failed"', async () => {
    const { feed, settle } = harness([bad('command list_version_conflict_center not found', true)])
    feed.start()
    await settle()
    expect(feed.state().reason).toBe(commandUnavailableLabel('list_version_conflict_center'))
  })

  test('a poll that fails after a good one keeps what the person was looking at, and stays ready', async () => {
    const { feed, settle, tick } = harness([ok([entry('a')]), bad('state.db is locked')])
    feed.start()
    await settle()
    tick()
    await settle()
    expect(feed.state().status).toBe('ready')
    expect(feed.state().entries.map((e) => e.id)).toEqual(['a'])
  })
})

describe('a poll that found nothing new does not wake the window', () => {
  test('the same answer twice: one notification and the very same state object', async () => {
    const { feed, settle, tick, seen } = harness([ok([entry('a')])])
    feed.start()
    await settle()
    const first = feed.state()
    tick()
    await settle()
    expect(feed.state()).toBe(first)
    expect(seen).toHaveLength(1)
  })

  test('a changed answer is a new state and a notification', async () => {
    const { feed, settle, tick, seen } = harness([ok([entry('a')]), ok([entry('a'), entry('b')])])
    feed.start()
    await settle()
    tick()
    await settle()
    expect(seen).toHaveLength(2)
    expect(feed.state().entries).toHaveLength(2)
  })

  test('an entry that only changed its detail line is a change', async () => {
    const { feed, settle, tick, seen } = harness([ok([entry('a', { detail: 'one' })]), ok([entry('a', { detail: 'two' })])])
    feed.start()
    await settle()
    tick()
    await settle()
    expect(seen).toHaveLength(2)
    expect(feed.state().entries[0].detail).toBe('two')
  })
})

describe('refresh', () => {
  test('reads at once, whatever the timer is doing, and its answer is shown', async () => {
    const { feed, reads, settle } = harness([ok([entry('a')]), ok([])])
    feed.start()
    await settle()
    feed.refresh()
    expect(reads).toHaveLength(2)
    await settle()
    expect(feed.state().entries).toEqual([])
  })

  test('an older read that lands after a refresh is dropped: the person just resolved that conflict', async () => {
    let releaseSlow: (r: CommandResult<VersionConflictEntry[]>) => void = () => {}
    const slow = new Promise<CommandResult<VersionConflictEntry[]>>((resolve) => { releaseSlow = resolve })
    const { feed, settle } = harness([slow, ok([])])
    feed.start() // read 1 is slow and still shows the conflict
    feed.refresh() // read 2 answers: nothing waits
    await settle()
    expect(feed.state().entries).toEqual([])
    releaseSlow(ok([entry('stale')]))
    await settle()
    expect(feed.state().entries).toEqual([])
  })

  test('before start it reads nothing', async () => {
    const { feed, reads } = harness([ok([])])
    feed.refresh()
    expect(reads).toHaveLength(0)
  })
})

describe('starting and stopping', () => {
  test('two windows or effects sharing the feed: it keeps polling until the last one stops, then clears its timer', async () => {
    const { feed, timers, tick, reads, settle } = harness([ok([])])
    const stopA = feed.start()
    const stopB = feed.start()
    expect(timers.size).toBe(1)
    stopA()
    expect(timers.size).toBe(1)
    tick()
    await settle()
    const before = reads.length
    stopB()
    expect(timers.size).toBe(0)
    feed.refresh()
    expect(reads).toHaveLength(before)
  })

  test('a stopped feed applies an answer that was still in flight to no one', async () => {
    let release: (r: CommandResult<VersionConflictEntry[]>) => void = () => {}
    const pending = new Promise<CommandResult<VersionConflictEntry[]>>((resolve) => { release = resolve })
    const { feed, seen, settle } = harness([pending])
    const stop = feed.start()
    stop()
    release(ok([entry('a')]))
    await settle()
    expect(seen).toHaveLength(0)
    expect(feed.state().status).toBe('loading')
  })

  test('a read still pending when the feed stopped never lands in the next lifetime: an account switch must not show the old account\'s rows (review thread on #142)', async () => {
    let releaseOld: (r: CommandResult<VersionConflictEntry[]>) => void = () => {}
    const old = new Promise<CommandResult<VersionConflictEntry[]>>((resolve) => { releaseOld = resolve })
    const { feed, seen, settle } = harness([old, ok([entry('new-account')])])
    const stopOld = feed.start() // the previous account's session: its read is slow
    stopOld() // signed out, or the vault locked
    feed.start() // the next session starts while that read is still out
    await settle()
    expect(feed.state().entries.map((e) => e.id)).toEqual(['new-account'])
    releaseOld(ok([entry('old-account')]))
    await settle()
    expect(feed.state().entries.map((e) => e.id)).toEqual(['new-account'])
    expect(seen.length).toBeGreaterThan(0)
  })

  test('stopping twice is harmless and does not stop the other holder', () => {
    const { feed, timers } = harness([ok([])])
    const stopA = feed.start()
    feed.start()
    stopA()
    stopA()
    expect(timers.size).toBe(1)
  })

  test('after a full stop the rows are gone: a vault that locks, or an account that is switched, never shows the last one\'s files', async () => {
    const { feed, timers, settle } = harness([ok([entry('a')]), ok([])])
    const stop = feed.start()
    await settle()
    expect(feed.state().entries).toHaveLength(1)
    stop()
    expect(timers.size).toBe(0)
    expect(feed.state()).toEqual({ status: 'loading', entries: [], reason: null })
    feed.start()
    expect(timers.size).toBe(1)
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', entries: [] })
  })
})

describe('connectReviewFeed: the moments that cannot wait for the timer', () => {
  function bus() {
    const handlers = new Map<string, () => void>()
    const removed: string[] = []
    const listen = (event: string, callback: () => void) => {
      handlers.set(event, callback)
      return Promise.resolve(() => { removed.push(event) })
    }
    return { listen, handlers, removed }
  }

  test('a conflict found by the engine and a conflict resolved both refresh at once', async () => {
    const { feed, reads, settle } = harness([ok([])])
    const { listen, handlers } = bus()
    connectReviewFeed(feed, listen)
    await settle()
    expect([...handlers.keys()].sort()).toEqual(['conflict-resolved', 'engine-conflict'])
    const before = reads.length
    handlers.get('engine-conflict')!()
    expect(reads).toHaveLength(before + 1)
    handlers.get('conflict-resolved')!()
    expect(reads).toHaveLength(before + 2)
  })

  test('the disposer stops the feed and removes both listeners', async () => {
    const { feed, timers, settle } = harness([ok([])])
    const { listen, removed } = bus()
    const dispose = connectReviewFeed(feed, listen)
    await settle()
    expect(timers.size).toBe(1)
    dispose()
    await settle()
    expect(timers.size).toBe(0)
    expect(removed.sort()).toEqual(['conflict-resolved', 'engine-conflict'])
  })

  test('disposed before the listeners were registered, they are removed the moment they arrive', async () => {
    const { feed, settle } = harness([ok([])])
    const removed: string[] = []
    const late = (event: string) => new Promise<() => void>((resolve) => { setTimeout(() => resolve(() => { removed.push(event) }), 0) })
    const dispose = connectReviewFeed(feed, (event: string) => late(event))
    dispose()
    await new Promise((resolve) => setTimeout(resolve, 5))
    await settle()
    expect(removed.sort()).toEqual(['conflict-resolved', 'engine-conflict'])
  })
})
