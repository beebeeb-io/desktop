/**
 * The Review feed (task 1683 slice 4; spec §7.6): one reader of `list_version_conflict_center` for the sidebar's badge
 * and the Review pane, so the two never disagree and the pane is not a second poller. The window reads it on its own
 * 5 s cadence while there is a session and an unlocked vault, and again at once when the engine finds a conflict or one
 * is resolved (`connectReviewFeed`).
 *
 * Rules, each pinned by tests/reviewFeed.test.ts:
 *  - a poll that found nothing new changes nothing (same state object, no notification), so a five-second timer never
 *    re-renders the window;
 *  - a poll that fails after a good one keeps what the person was looking at; a failure is shown only when there is
 *    nothing to show;
 *  - an older read that lands after a refresh is dropped (`SnapshotReader`, the window's snapshot reader);
 *  - when the last holder stops, the rows are dropped: a locked vault, or another account, never sees the last one's files.
 */
import { command, commandUnavailableLabel, type CommandResult, type VersionConflictEntry } from './desktopApi'
import { SnapshotReader } from './mainWindowModel'
import { parseReviewEntries } from './reviewModel'

export const REVIEW_FEED_INTERVAL_MS = 5000

export interface ReviewFeedState {
  status: 'loading' | 'ready' | 'failed'
  entries: VersionConflictEntry[]
  /** Why the first read failed; null otherwise. */
  reason: string | null
}

const LOADING: ReviewFeedState = { status: 'loading', entries: [], reason: null }

interface Clock {
  setInterval(callback: () => void, ms: number): number
  clearInterval(id: number): void
}

const windowClock: Clock = {
  setInterval: (callback, ms) => window.setInterval(callback, ms),
  clearInterval: (id) => window.clearInterval(id),
}

type Read = () => Promise<CommandResult<VersionConflictEntry[]>>

export class ReviewFeed {
  private current: ReviewFeedState = LOADING
  private readonly listeners = new Set<() => void>()
  private readonly reader: SnapshotReader<CommandResult<VersionConflictEntry[]>>
  private holders = 0
  private timer: number | null = null

  constructor(read: Read, private readonly clock: Clock = windowClock) {
    this.reader = new SnapshotReader(read, (result) => this.apply(result))
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

  /** Starts reading (at once, then on the timer) and returns the way to stop. The feed runs until every holder stopped. */
  start(): () => void {
    this.holders += 1
    if (this.holders === 1) {
      this.reader.resume()
      this.reader.tick()
      this.timer = this.clock.setInterval(() => this.reader.tick(), REVIEW_FEED_INTERVAL_MS)
    }
    let stopped = false
    return () => {
      if (stopped) return
      stopped = true
      this.holders -= 1
      if (this.holders === 0) this.halt()
    }
  }

  /** Reads now, whatever the timer is doing. Nothing before `start`. */
  refresh(): void {
    this.reader.refresh()
  }

  private halt(): void {
    this.reader.stop()
    if (this.timer !== null) this.clock.clearInterval(this.timer)
    this.timer = null
    this.set(LOADING)
  }

  private apply(result: CommandResult<VersionConflictEntry[]>): void {
    if (result.ok) {
      this.set({ status: 'ready', entries: result.value, reason: null })
      return
    }
    // A poll that fails after a good one keeps what is shown; the next good one heals it.
    if (this.current.status === 'ready') return
    const failure = result as { reason?: string; unsupported?: boolean }
    this.set({
      status: 'failed',
      entries: [],
      reason: failure.unsupported ? commandUnavailableLabel('list_version_conflict_center') : failure.reason ?? 'Couldn’t read Review.',
    })
  }

  private set(next: ReviewFeedState): void {
    const same =
      next.status === this.current.status &&
      next.reason === this.current.reason &&
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
export async function readReviewEntries(): Promise<CommandResult<VersionConflictEntry[]>> {
  const result = await command<unknown>('list_version_conflict_center')
  if (!result.ok) return result
  const entries = parseReviewEntries(result.value)
  return entries === null
    ? { ok: false, reason: 'Review answered in a form this version of Beebeeb cannot read.', unsupported: false }
    : { ok: true, value: entries }
}

/** The window's one feed. */
export const reviewFeed = new ReviewFeed(readReviewEntries)
export const subscribeReviewFeed = reviewFeed.subscribe
export const reviewFeedState = reviewFeed.state
export const refreshReviewFeed = (): void => reviewFeed.refresh()
