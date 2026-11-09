// The RAW strip's thumbnails (DW-1501): blob URLs kept by item id across runs and steps (the open file remounts per
// step), and bounded. A URL is freed only when it is replaced or pushed out, never while the strip may still show it
// (desktop #273 finding 4: a dependency change revoked every URL the strip was drawing).
import type { DriveSource } from '@beebeeb/shared/drive-ui'

export type ThumbSize = 'large' | 'small'

export class ThumbCache {
  private map = new Map<string, { url: string | null; size: ThumbSize }>()
  constructor(private readonly cap: number, private readonly revoke: (url: string) => void = (u) => URL.revokeObjectURL(u)) {}

  /** The URL held for `id` (null: the item has none) when it is at least `size`; undefined when it must be fetched. */
  get(id: string, size: ThumbSize): string | null | undefined {
    const e = this.map.get(id)
    if (!e || (size === 'large' && e.size === 'small')) return undefined
    this.map.delete(id)
    this.map.set(id, e) // the most recently used is pushed out last
    return e.url
  }

  /** Keep `url` for `id`. One already held at this size or larger stays (a late answer frees its own URL, never the one
   *  on screen); a small one is replaced by a large one and freed; past the cap the least recently used is freed. */
  put(id: string, size: ThumbSize, url: string | null): void {
    const old = this.map.get(id)
    if (old && (old.size === 'large' || size === 'small')) { if (url && url !== old.url) this.revoke(url); return }
    this.map.delete(id)
    this.map.set(id, { url, size })
    if (old?.url && old.url !== url) this.revoke(old.url)
    while (this.map.size > this.cap) {
      const [first, e] = this.map.entries().next().value as [string, { url: string | null }]
      this.map.delete(first)
      if (e.url) this.revoke(e.url)
    }
  }

  get size(): number { return this.map.size }
}

/** Room for the strip on both sides of a step and the steps around it. */
export const RAW_THUMBS_HELD = 48
const caches = new WeakMap<DriveSource, ThumbCache>()
/** One cache per source: another account's source never reads this one's thumbnails. */
export function rawThumbCache(source: DriveSource): ThumbCache {
  let c = caches.get(source)
  if (!c) caches.set(source, (c = new ThumbCache(RAW_THUMBS_HELD)))
  return c
}
