/**
 * The Review feed (task 1683 slice 4; spec §7.6): one reader of Review for the sidebar's badge and the Review pane, so the
 * two never disagree and the pane is not a second poller. The window reads it on its own 5 s cadence while there is a
 * session and an unlocked vault, and again at once when the engine finds a conflict or one is resolved
 * (`connectReviewFeed`).
 *
 * Two reads, because they cost differently (review of #142): the COUNT (`review_item_count`, one SQL count) is what the
 * badge needs and is read all the time; the LIST (`list_version_conflict_center`, at most 200 rows, each built from the
 * state database) is read only while something shows it (`watchList`: the Review pane). The list can hold fewer rows than
 * the count says wait (`ReviewFeedState.count`), and the pane says so.
 *
 * Rules, each pinned by tests/reviewFeed.test.ts:
 *  - a poll that found nothing new changes nothing (same state object, no notification), so a five-second timer never
 *    re-renders the window;
 *  - a poll that fails after a good one keeps what the person was looking at; a failure is shown only when there is
 *    nothing to show;
 *  - an older read that lands after a refresh is dropped (`SnapshotReader`, the window's snapshot reader);
 *  - when the last holder stops, everything is dropped: a locked vault, or another account, never sees the last one's
 *    files or count; when the last list watcher stops, the rows are dropped (the count stays), so a pane that opens again
 *    never shows rows from before it closed.
 */
import { command, commandUnavailableLabel, type CommandResult, type VersionConflictEntry } from './desktopApi'
import { SnapshotReader } from './mainWindowModel'
import { parseReviewFeed } from './reviewModel'

export const REVIEW_FEED_INTERVAL_MS = 5000

export interface ReviewFeedState {
  /** Of the LIST: loading until the first read of it lands, and only while something watches it. */
  status: 'loading' | 'ready' | 'failed'
  entries: VersionConflictEntry[]
  /** How many things wait in all (the badge); 0 until the count has been read. More than `entries` when the feed is capped. */
  count: number
  /** Entries the list carried that this version of Beebeeb could not read: they are not in `entries`. */
  rejected: number
  /** Why the first read of the list failed; null otherwise. */
  reason: string | null
}

const LOADING: ReviewFeedState = { status: 'loading', entries: [], count: 0, rejected: 0, reason: null }

interface Clock {
  setInterval(callback: () => void, ms: number): number
  clearInterval(id: number): void
}

const windowClock: Clock = {
  setInterval: (callback, ms) => window.setInterval(callback, ms),
  clearInterval: (id) => window.clearInterval(id),
}

/** What the list read returns: the rows that read, and how many did not. */
export interface ReviewRead {
  entries: VersionConflictEntry[]
  rejected: number
}

export interface ReviewReads {
  count: () => Promise<CommandResult<number>>
  list: () => Promise<CommandResult<ReviewRead>>
}

export class ReviewFeed {
  private current: ReviewFeedState = LOADING
  private readonly listeners = new Set<() => void>()
  private readonly countReader: SnapshotReader<CommandResult<number>>
  private readonly listReader: SnapshotReader<CommandResult<ReviewRead>>
  /** The count as the count read answered it; `ReviewFeedState.count` is never less than the rows it carries. */
  private counted = 0
  private holders = 0
  private watchers = 0
  private listRunning = false
  private timer: number | null = null

  constructor(
    reads: ReviewReads,
    private readonly clock: Clock = windowClock,
  ) {
    // The readers are reused by every lifetime of the feed (a lock, an account switch), and `discard` drops every read a
    // reader still has out, so one that outlives its lifetime can land in no later one.
    this.countReader = new SnapshotReader(reads.count, (result) => this.applyCount(result))
    this.listReader = new SnapshotReader(reads.list, (result) => this.applyList(result))
  }

  /** `useSyncExternalStore`'s `getSnapshot`: the same object until something changed. */
  state = (): ReviewFeedState => this.current

  /** `useSyncExternalStore`'s `subscribe`. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  /** Starts reading the count (at once, then on the timer) and returns the way to stop. It runs until every holder stopped. */
  start(): () => void {
    this.holders += 1
    if (this.holders === 1) {
      this.countReader.resume()
      this.countReader.tick()
      this.timer = this.clock.setInterval(() => this.tick(), REVIEW_FEED_INTERVAL_MS)
      this.syncList()
    }
    let stopped = false
    return () => {
      if (stopped) return
      stopped = true
      this.holders -= 1
      if (this.holders === 0) this.halt()
    }
  }

  /**
   * Asks for the list too, for as long as the returned function has not been called: the Review pane. It reads at once
   * (when the feed is running; a watch before `start` waits for it) and then on the timer.
   */
  watchList(): () => void {
    this.watchers += 1
    this.syncList()
    let stopped = false
    return () => {
      if (stopped) return
      stopped = true
      this.watchers -= 1
      this.syncList()
    }
  }

  /** Reads now, whatever the timer is doing: the count, and the list if something watches it. Nothing before `start`. */
  refresh(): void {
    this.countReader.refresh()
    if (this.listRunning) this.listReader.refresh()
  }

  private tick(): void {
    this.countReader.tick()
    if (this.listRunning) this.listReader.tick()
  }

  /** The list is read exactly while the feed runs and something watches it. */
  private syncList(): void {
    const wanted = this.holders > 0 && this.watchers > 0
    if (wanted && !this.listRunning) {
      this.listRunning = true
      this.listReader.resume()
      this.listReader.tick()
    } else if (!wanted && this.listRunning) {
      this.listRunning = false
      this.listReader.discard()
      this.set({ ...this.current, status: 'loading', entries: [], count: this.counted, rejected: 0, reason: null })
    }
  }

  private halt(): void {
    this.syncList()
    this.countReader.discard()
    if (this.timer !== null) this.clock.clearInterval(this.timer)
    this.timer = null
    this.counted = 0
    this.set(LOADING)
  }

  private applyCount(result: CommandResult<number>): void {
    // A count that fails keeps the one shown; the next good one heals it.
    if (!result.ok) return
    this.counted = result.value
    this.set({ ...this.current, count: Math.max(result.value, this.current.entries.length) })
  }

  private applyList(result: CommandResult<ReviewRead>): void {
    if (result.ok) {
      // The count is read on its own clock and can be a beat behind the list; the rows shown are never more than wait.
      const count = Math.max(this.counted, result.value.entries.length)
      this.set({ status: 'ready', entries: result.value.entries, count, rejected: result.value.rejected, reason: null })
      return
    }
    // A poll that fails after a good one keeps what is shown; the next good one heals it.
    if (this.current.status === 'ready') return
    const failure = result as { reason?: string; unsupported?: boolean }
    this.set({
      ...this.current,
      status: 'failed',
      entries: [],
      rejected: 0,
      reason: failure.unsupported ? commandUnavailableLabel('list_version_conflict_center') : failure.reason ?? 'Couldn’t read Review.',
    })
  }

  private set(next: ReviewFeedState): void {
    const same =
      next.status === this.current.status &&
      next.reason === this.current.reason &&
      next.count === this.current.count &&
      next.rejected === this.current.rejected &&
      JSON.stringify(next.entries) === JSON.stringify(this.current.entries)
    if (same) return
    this.current = next
    for (const listener of [...this.listeners]) listener()
  }
}

type Listen = (event: string, callback: () => void) => Promise<() => void>

/** The moments that cannot wait for the timer: the engine found a conflict, or one was just resolved. */
const REFRESH_EVENTS = ['engine-conflict', 'conflict-resolved'] as const

/**
 * Starts `feed` and keeps it fresh on the two events above. The disposer stops the feed and removes both listeners,
 * also when it runs before the listeners were registered (the pattern of the window's other event connections).
 */
export function connectReviewFeed(feed: ReviewFeed, listen: Listen): () => void {
  const stop = feed.start()
  let disposed = false
  const removers: Array<() => void> = []
  for (const event of REFRESH_EVENTS) {
    void listen(event, () => feed.refresh()).then((remove) => {
      if (disposed) remove()
      else removers.push(remove)
    })
  }
  return () => {
    if (disposed) return
    disposed = true
    stop()
    for (const remove of removers) remove()
  }
}

/** `list_version_conflict_center`, read through the entry validator: an answer that is not a feed is a failure, not rows. */
export async function readReviewEntries(): Promise<CommandResult<ReviewRead>> {
  const result = await command<unknown>('list_version_conflict_center')
  if (!result.ok) return result
  const feed = parseReviewFeed(result.value)
  return feed === null
    ? { ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false }
    : { ok: true, value: feed }
}

/** `review_item_count`: how many things wait, as one number. Anything else is a failure, not a count of zero. */
export async function readReviewCount(): Promise<CommandResult<number>> {
  const result = await command<unknown>('review_item_count')
  if (!result.ok) return result
  return typeof result.value === 'number' && Number.isSafeInteger(result.value) && result.value >= 0
    ? { ok: true, value: result.value }
    : { ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false }
}

/** The window's one feed. */
export const reviewFeed = new ReviewFeed({ count: readReviewCount, list: readReviewEntries })
export const subscribeReviewFeed = reviewFeed.subscribe
export const reviewFeedState = reviewFeed.state
export const refreshReviewFeed = (): void => reviewFeed.refresh()
