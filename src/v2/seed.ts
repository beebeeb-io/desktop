// Desktop v2's live state route (`?state=DW-xxxx`, dev server only; recipes in ./dev/): a component's first UI state can
// come from the frame's recipe, so the real component, with its real handlers and routing, opens in that frame's state.
// In a production build `import.meta.env.DEV` is false: `seed` returns the fallback, and ./dev is never bundled.

const SEED_KEY = '__bbV2Seed'

/** The recipe's value for `key` while a live state is being opened, else `fallback`. */
export function seed<T>(key: string, fallback: T): T {
  if (!import.meta.env.DEV) return fallback
  const seeds = (globalThis as Record<string, unknown>)[SEED_KEY] as Record<string, unknown> | undefined
  return seeds && key in seeds ? (seeds[key] as T) : fallback
}

/** Dev only: set (or with null, clear) the seeds. Cleared once the state is open, so a later remount starts plain. */
export function setSeeds(seeds: Record<string, unknown> | null): void {
  if (!import.meta.env.DEV) return
  if (seeds) (globalThis as Record<string, unknown>)[SEED_KEY] = seeds
  else delete (globalThis as Record<string, unknown>)[SEED_KEY]
}
