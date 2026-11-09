//! A function that runs at once, then at most once per window (the sidebar counts use it).
/** Runs `fn` now, then at most once per `ms`: calls inside the window collapse into one run at its end. */
export function throttled(fn: () => void, ms = 1000): (() => void) & { cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | undefined
  let again = false
  const run = () => {
    if (timer) { again = true; return }
    fn()
    timer = setTimeout(() => { timer = undefined; if (again) { again = false; run() } }, ms)
  }
  return Object.assign(run, { cancel: () => { clearTimeout(timer); timer = undefined; again = false } })
}

/**
 * Which read is the newest: each call starts one and returns its check, true while no later one has started. An answer
 * that comes back after a later read started is older than what is on screen, and is dropped (#274 review).
 */
export function newestOnly(): () => () => boolean {
  let n = 0
  return () => { const mine = ++n; return () => mine === n }
}
