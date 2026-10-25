/**
 * The Review feed (task 1683 slice 4; spec §7.6): one place reads Review for the sidebar badge and the Review pane, on
 * the window's 5 s cadence and again the moment a conflict is found or resolved. Two reads, because they cost differently
 * (review of #142): the COUNT (`review_item_count`) is read all the time and is the badge; the LIST
 * (`list_version_conflict_center`, capped by Rust) is read only while something watches it.
 * What this proves: the states a reader can be in, that a poll never churns the window when nothing changed, that a
 * failed poll never wipes what was already shown, that the list is read only while watched, that rows read from one
 * lifetime never land in the next, and that stopping really stops. What it does NOT prove: that Rust's feed is right
 * (src-tauri tests) or the window around it (tests/macApp.test.tsx).
 */
import { describe, expect, test } from 'bun:test'
import { commandUnavailableLabel, type CommandResult, type VersionConflictEntry } from '../src/desktopApi'
import { connectReviewFeed, readReviewCount, readReviewEntries, ReviewFeed, REVIEW_FEED_INTERVAL_MS, type ReviewRead } from '../src/reviewFeed'

const entry = (id: string, over: Partial<VersionConflictEntry> = {}): VersionConflictEntry => ({
  id, file_id: `f-${id}`, file_name: `${id}.md`, kind: 'conflict', status: 'needs review', updated_at: 1000, detail: 'd', action: 'open_conflict', ...over,
})

type Count = CommandResult<number>
type List = CommandResult<ReviewRead>
const count = (n: number): Count => ({ ok: true, value: n })
const list = (entries: VersionConflictEntry[], rejected = 0): List => ({ ok: true, value: { entries, rejected } })
const bad = (reason: string, unsupported = false) => ({ ok: false, reason, unsupported }) as CommandResult<never>
/** The state of a feed that has read nothing. */
const NOTHING = { status: 'loading', entries: [], count: 0, rejected: 0, reason: null }

/** Scripted readers (the last answer repeats) and a clock we turn by hand. */
function harness(script: { counts?: Array<Count | Promise<Count>>; lists?: Array<List | Promise<List>> }) {
  const countScript = script.counts ?? [count(0)]
  const listScript = script.lists ?? [list([])]
  const countReads: number[] = []
  const listReads: number[] = []
  const reads = {
    count: () => { countReads.push(countReads.length); return Promise.resolve(countScript[Math.min(countReads.length - 1, countScript.length - 1)]) },
    list: () => { listReads.push(listReads.length); return Promise.resolve(listScript[Math.min(listReads.length - 1, listScript.length - 1)]) },
  }
  const timers = new Map<number, () => void>()
  let nextId = 1
  const clock = {
    setInterval: (fn: () => void, ms: number) => { expect(ms).toBe(REVIEW_FEED_INTERVAL_MS); const id = nextId++; timers.set(id, fn); return id },
    clearInterval: (id: number) => { timers.delete(id) },
  }
  const feed = new ReviewFeed(reads, clock)
  const seen: unknown[] = []
  feed.subscribe(() => seen.push(feed.state()))
  const tick = () => { for (const fn of [...timers.values()]) fn() }
  const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }
  return { feed, countReads, listReads, timers, tick, settle, seen }
}

describe('the cadence is the window\'s own', () => {
  test('5 seconds, like the snapshot read', () => {
    expect(REVIEW_FEED_INTERVAL_MS).toBe(5000)
  })
})

describe('the count (the badge)', () => {
  test('before the first answer there is nothing: count 0, loading', () => {
    const { feed } = harness({})
    expect(feed.state()).toEqual(NOTHING)
  })

  test('starting reads the count at once and then on the timer, and not the list: nothing watches it', async () => {
    const { feed, countReads, listReads, settle, tick } = harness({ counts: [count(3)] })
    feed.start()
    expect(countReads).toHaveLength(1)
    await settle()
    expect(feed.state().count).toBe(3)
    tick()
    expect(countReads).toHaveLength(2)
    expect(listReads).toHaveLength(0)
    expect(feed.state().status).toBe('loading')
  })

  test('a count that fails keeps the one shown; before any it stays 0 and the list\'s state is untouched', async () => {
    const { feed, settle, tick } = harness({ counts: [bad('state.db is locked'), count(2), bad('state.db is locked')] })
    feed.start()
    await settle()
    expect(feed.state()).toEqual(NOTHING)
    tick()
    await settle()
    expect(feed.state().count).toBe(2)
    tick()
    await settle()
    expect(feed.state().count).toBe(2)
  })

  test('the same count twice: one notification and the very same state object', async () => {
    const { feed, settle, tick, seen } = harness({ counts: [count(1)] })
    feed.start()
    await settle()
    const first = feed.state()
    tick()
    await settle()
    expect(feed.state()).toBe(first)
    expect(seen).toHaveLength(1)
  })
})

describe('the list (the pane), read only while watched', () => {
  test('watching reads the list at once and on the timer; the answer is ready, in the order Rust sent it', async () => {
    const { feed, listReads, settle, tick } = harness({ lists: [list([entry('a'), entry('b')])] })
    feed.start()
    feed.watchList()
    expect(listReads).toHaveLength(1)
    await settle()
    expect(feed.state().status).toBe('ready')
    expect(feed.state().entries.map((e) => e.id)).toEqual(['a', 'b'])
    tick()
    expect(listReads).toHaveLength(2)
  })

  test('a watch before the feed started waits for it, and reads as soon as it has', async () => {
    const { feed, listReads, settle } = harness({ lists: [list([entry('a')])] })
    feed.watchList()
    expect(listReads).toHaveLength(0)
    feed.start()
    await settle()
    expect(listReads).toHaveLength(1)
    expect(feed.state().status).toBe('ready')
  })

  test('letting go of the last watch stops reading the list and drops its rows; the count stays', async () => {
    const { feed, listReads, settle, tick } = harness({ counts: [count(5)], lists: [list([entry('a')])] })
    feed.start()
    const unwatch = feed.watchList()
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', count: 5 })
    unwatch()
    expect(feed.state()).toEqual({ status: 'loading', entries: [], count: 5, rejected: 0, reason: null })
    const before = listReads.length
    tick()
    expect(listReads).toHaveLength(before)
  })

  test('two watchers: the list is read until the last lets go; letting go twice is harmless', async () => {
    const { feed, listReads, settle, tick } = harness({ lists: [list([entry('a')])] })
    feed.start()
    const a = feed.watchList()
    const b = feed.watchList()
    await settle()
    a()
    a()
    tick()
    expect(listReads).toHaveLength(2)
    b()
    tick()
    expect(listReads).toHaveLength(2)
  })

  test('a pane that opens again never shows the rows it showed before it closed', async () => {
    const { feed, settle } = harness({ lists: [list([entry('old')]), list([entry('new')])] })
    feed.start()
    const first = feed.watchList()
    await settle()
    expect(feed.state().entries.map((e) => e.id)).toEqual(['old'])
    first()
    feed.watchList()
    expect(feed.state().entries).toEqual([])
    expect(feed.state().status).toBe('loading')
    await settle()
    expect(feed.state().entries.map((e) => e.id)).toEqual(['new'])
  })

  test('a first read that fails is "failed" with the reason and the count kept; the next good read heals it', async () => {
    const { feed, settle, tick } = harness({ counts: [count(4)], lists: [bad('state.db is locked'), list([entry('a')])] })
    feed.start()
    feed.watchList()
    await settle()
    expect(feed.state()).toEqual({ status: 'failed', entries: [], count: 4, rejected: 0, reason: 'state.db is locked' })
    tick()
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', reason: null, count: 4 })
    expect(feed.state().entries).toHaveLength(1)
  })

  test('a build without the command says so in the shared words, not "invoke failed"', async () => {
    const { feed, settle } = harness({ lists: [bad('command list_version_conflict_center not found', true)] })
    feed.start()
    feed.watchList()
    await settle()
    expect(feed.state().reason).toBe(commandUnavailableLabel('list_version_conflict_center'))
  })

  test('a poll that fails after a good one keeps what the person was looking at, and stays ready', async () => {
    const { feed, settle, tick } = harness({ lists: [list([entry('a')]), bad('state.db is locked')] })
    feed.start()
    feed.watchList()
    await settle()
    tick()
    await settle()
    expect(feed.state().status).toBe('ready')
    expect(feed.state().entries.map((e) => e.id)).toEqual(['a'])
  })

  test('entries the answer carried that did not read are counted, not dropped without a word', async () => {
    const { feed, settle } = harness({ counts: [count(3)], lists: [list([entry('a')], 2)] })
    feed.start()
    feed.watchList()
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', rejected: 2, count: 3 })
    expect(feed.state().entries).toHaveLength(1)
  })

  test('the count can be more than the rows (the list is capped), and never fewer: a list longer than a lagging count wins', async () => {
    const capped = harness({ counts: [count(250)], lists: [list([entry('a'), entry('b')])] })
    capped.feed.start()
    capped.feed.watchList()
    await capped.settle()
    expect(capped.feed.state().count).toBe(250)
    expect(capped.feed.state().entries).toHaveLength(2)
    const lagging = harness({ counts: [count(1)], lists: [list([entry('a'), entry('b'), entry('c')])] })
    lagging.feed.start()
    lagging.feed.watchList()
    await lagging.settle()
    expect(lagging.feed.state().count).toBe(3)
  })

  test('that floor is not sticky: once the count catches up to a smaller number, so does the state', async () => {
    const { feed, settle, tick } = harness({ counts: [count(3), count(1)], lists: [list([entry('a'), entry('b'), entry('c')]), list([entry('a')])] })
    feed.start()
    feed.watchList()
    await settle()
    expect(feed.state().count).toBe(3)
    tick()
    await settle()
    expect(feed.state().count).toBe(1)
    expect(feed.state().entries).toHaveLength(1)
  })
})

describe('a poll that found nothing new does not wake the window', () => {
  test('the same list twice: one notification and the very same state object', async () => {
    const { feed, settle, tick, seen } = harness({ lists: [list([entry('a')])] })
    feed.start()
    feed.watchList()
    await settle()
    const first = feed.state()
    const notified = seen.length
    tick()
    await settle()
    expect(feed.state()).toBe(first)
    expect(seen).toHaveLength(notified)
  })

  test('a changed answer is a new state and a notification', async () => {
    const { feed, settle, tick, seen } = harness({ lists: [list([entry('a')]), list([entry('a'), entry('b')])] })
    feed.start()
    feed.watchList()
    await settle()
    const notified = seen.length
    tick()
    await settle()
    expect(seen).toHaveLength(notified + 1)
    expect(feed.state().entries).toHaveLength(2)
  })

  test('an entry that only changed its detail line is a change', async () => {
    const { feed, settle, tick } = harness({ lists: [list([entry('a', { detail: 'one' })]), list([entry('a', { detail: 'two' })])] })
    feed.start()
    feed.watchList()
    await settle()
    tick()
    await settle()
    expect(feed.state().entries[0].detail).toBe('two')
  })

  test('a change in how many did not read is a change', async () => {
    const { feed, settle, tick } = harness({ lists: [list([entry('a')], 0), list([entry('a')], 1)] })
    feed.start()
    feed.watchList()
    await settle()
    tick()
    await settle()
    expect(feed.state().rejected).toBe(1)
  })
})

describe('refresh', () => {
  test('reads the count at once, and the list too when it is watched, whatever the timer is doing', async () => {
    const { feed, countReads, listReads, settle } = harness({ lists: [list([entry('a')]), list([])] })
    feed.start()
    await settle()
    feed.refresh()
    expect(countReads).toHaveLength(2)
    expect(listReads).toHaveLength(0)
    feed.watchList()
    await settle()
    feed.refresh()
    expect(listReads).toHaveLength(2)
    await settle()
    expect(feed.state().entries).toEqual([])
  })

  test('an older read that lands after a refresh is dropped: the person just resolved that conflict', async () => {
    let releaseSlow: (r: List) => void = () => {}
    const slow = new Promise<List>((resolve) => { releaseSlow = resolve })
    const { feed, settle } = harness({ lists: [slow, list([])] })
    feed.start()
    feed.watchList() // read 1 is slow and still shows the conflict
    feed.refresh() // read 2 answers: nothing waits
    await settle()
    expect(feed.state().entries).toEqual([])
    releaseSlow(list([entry('stale')]))
    await settle()
    expect(feed.state().entries).toEqual([])
  })

  test('before start it reads nothing', async () => {
    const { feed, countReads, listReads } = harness({})
    feed.refresh()
    expect(countReads).toHaveLength(0)
    expect(listReads).toHaveLength(0)
  })
})

describe('starting and stopping', () => {
  test('two windows or effects sharing the feed: it keeps polling until the last one stops, then clears its timer', async () => {
    const { feed, timers, tick, countReads, settle } = harness({})
    const stopA = feed.start()
    const stopB = feed.start()
    expect(timers.size).toBe(1)
    stopA()
    expect(timers.size).toBe(1)
    tick()
    await settle()
    const before = countReads.length
    stopB()
    expect(timers.size).toBe(0)
    feed.refresh()
    expect(countReads).toHaveLength(before)
  })

  test('a stopped feed applies an answer that was still in flight to no one', async () => {
    let release: (r: Count) => void = () => {}
    const pending = new Promise<Count>((resolve) => { release = resolve })
    const { feed, seen, settle } = harness({ counts: [pending] })
    const stop = feed.start()
    stop()
    release(count(9))
    await settle()
    expect(seen).toHaveLength(0)
    expect(feed.state()).toEqual(NOTHING)
  })

  test('a count or a list still pending when the feed stopped never lands in the next lifetime: an account switch must not show the old account\'s rows (review thread on #142)', async () => {
    let releaseOldCount: (r: Count) => void = () => {}
    let releaseOldList: (r: List) => void = () => {}
    const oldCount = new Promise<Count>((resolve) => { releaseOldCount = resolve })
    const oldList = new Promise<List>((resolve) => { releaseOldList = resolve })
    const { feed, seen, settle } = harness({ counts: [oldCount, count(1)], lists: [oldList, list([entry('new-account')])] })
    const stopOld = feed.start() // the previous account's session: its reads are slow
    feed.watchList()
    stopOld() // signed out, or the vault locked
    feed.start() // the next session starts while those reads are still out
    await settle()
    expect(feed.state().count).toBe(1)
    expect(feed.state().entries.map((e) => e.id)).toEqual(['new-account'])
    releaseOldCount(count(40))
    releaseOldList(list([entry('old-account')]))
    await settle()
    expect(feed.state().count).toBe(1)
    expect(feed.state().entries.map((e) => e.id)).toEqual(['new-account'])
    expect(seen.length).toBeGreaterThan(0)
  })

  test('stopping twice is harmless and does not stop the other holder', () => {
    const { feed, timers } = harness({})
    const stopA = feed.start()
    feed.start()
    stopA()
    stopA()
    expect(timers.size).toBe(1)
  })

  test('after a full stop everything is gone: a vault that locks, or an account that is switched, never shows the last one\'s files or number', async () => {
    const { feed, timers, settle } = harness({ counts: [count(2), count(0)], lists: [list([entry('a')]), list([])] })
    const stop = feed.start()
    feed.watchList()
    await settle()
    expect(feed.state()).toMatchObject({ count: 2 })
    expect(feed.state().entries).toHaveLength(1)
    stop()
    expect(timers.size).toBe(0)
    expect(feed.state()).toEqual(NOTHING)
    feed.start()
    expect(timers.size).toBe(1)
    await settle()
    expect(feed.state()).toMatchObject({ status: 'ready', entries: [], count: 0 })
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
    const { feed, countReads, settle } = harness({})
    const { listen, handlers } = bus()
    connectReviewFeed(feed, listen)
    await settle()
    expect([...handlers.keys()].sort()).toEqual(['conflict-resolved', 'engine-conflict'])
    const before = countReads.length
    handlers.get('engine-conflict')!()
    expect(countReads).toHaveLength(before + 1)
    handlers.get('conflict-resolved')!()
    expect(countReads).toHaveLength(before + 2)
  })

  test('the disposer stops the feed and removes both listeners', async () => {
    const { feed, timers, settle } = harness({})
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
    const { feed, settle } = harness({})
    const removed: string[] = []
    const late = (event: string) => new Promise<() => void>((resolve) => { setTimeout(() => resolve(() => { removed.push(event) }), 0) })
    const dispose = connectReviewFeed(feed, (event: string) => late(event))
    dispose()
    await new Promise((resolve) => setTimeout(resolve, 5))
    await settle()
    expect(removed.sort()).toEqual(['conflict-resolved', 'engine-conflict'])
  })
})

describe('the two readers go through the validator', () => {
  const withBackend = async <T>(answers: Record<string, () => unknown>, run: () => Promise<T>) => {
    const previous = (globalThis as any).window
    ;(globalThis as any).window = {
      __TAURI_INTERNALS__: { invoke: async (name: string) => { if (!answers[name]) throw new Error(`unscripted ${name}`); return answers[name]() } },
    }
    try { return await run() } finally { (globalThis as any).window = previous }
  }

  test('a count is a whole number of zero or more; anything else is a failure, never a count of zero', async () => {
    const good = await withBackend({ review_item_count: () => 7 }, readReviewCount)
    expect(good).toEqual({ ok: true, value: 7 })
    for (const answer of [-1, 1.5, '7', null, undefined, {}, NaN, Number.MAX_SAFE_INTEGER + 2]) {
      const result = await withBackend({ review_item_count: () => answer }, readReviewCount)
      expect(result).toEqual({ ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false })
    }
  })

  test('the list keeps the rows that read and says how many did not', async () => {
    const good = { id: 'op:1', file_id: 'f', file_name: 'a.md', kind: 'conflict', status: 's', detail: 'd', action: 'open_conflict' }
    const result = await withBackend({ list_version_conflict_center: () => [good, { id: 'x' }, null, good] }, readReviewEntries)
    expect(result.ok && { rows: result.value.entries.length, rejected: result.value.rejected }).toEqual({ rows: 2, rejected: 2 })
  })

  test('an answer that is not a list is a failure the pane can show, not an empty Review', async () => {
    const result = await withBackend({ list_version_conflict_center: () => ({ unexpected: true }) }, readReviewEntries)
    expect(result).toEqual({ ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false })
  })

  test('a command failure is passed through untouched, for both', async () => {
    const list = await withBackend({ list_version_conflict_center: () => { throw new Error('state.db is locked') } }, readReviewEntries)
    expect(!list.ok && list.reason).toBe('state.db is locked')
    const counted = await withBackend({ review_item_count: () => { throw new Error('state.db is locked') } }, readReviewCount)
    expect(!counted.ok && counted.reason).toBe('state.db is locked')
  })
})
