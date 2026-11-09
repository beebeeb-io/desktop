// The board's example data, for a window with no host behind it (P1c). On the dev server in a plain browser, the live
// state route (./dev/live.ts) registers the board's fixtures here before a window mounts, so the frames and the browser
// render the board's pages. Everywhere else nothing is registered: in a production build `import.meta.env.DEV` is
// false and every lookup returns undefined, and inside Tauri (where ./dev never loads) the real backend answers. A
// production module never imports the fixtures themselves (tests/v2NoFixturesInProduction.test.ts).

const BOARD_KEY = '__bbV2Board'

/** Dev only: the board's values by name. */
export function registerBoard(values: Record<string, unknown>): void {
  if (!import.meta.env.DEV) return
  ;(globalThis as Record<string, unknown>)[BOARD_KEY] = values
}

/** The board's value for `key` on the dev server in a plain browser; undefined in the running app. */
export function board<T>(key: string): T | undefined {
  if (!import.meta.env.DEV) return undefined
  const values = (globalThis as Record<string, unknown>)[BOARD_KEY] as Record<string, unknown> | undefined
  return values && key in values ? (values[key] as T) : undefined
}
