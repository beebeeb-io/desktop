// Dev only (batch M3a), never bundled: the live state route opens a frame's state at the board's clock. The frames are
// drawn on 2026-11-08 at 12:00 (the fixtures' NOW), and the fixtures' times are absolute, so a live screen read on
// another day says "7 Nov 2026" where its frame says "Yesterday, 16:42". Pinning `Date` to that moment (it keeps
// running from there) makes a live state read the same on any day. Timers are untouched: only `Date` moves.

export const BOARD_NOW = '2026-11-08T12:00:00'

export function pinClock(at: string = BOARD_NOW): void {
  const Real = Date
  const offset = new Real(at).getTime() - Real.now()
  function BoardDate(this: unknown, ...args: unknown[]): unknown {
    // Called without `new`, Date returns the time as a string.
    if (!new.target) return new Real(Real.now() + offset).toString()
    return args.length ? Reflect.construct(Real, args) : new Real(Real.now() + offset)
  }
  BoardDate.prototype = Real.prototype
  Object.assign(BoardDate, { now: () => Real.now() + offset, parse: Real.parse, UTC: Real.UTC })
  ;(globalThis as { Date: DateConstructor }).Date = BoardDate as unknown as DateConstructor
}
