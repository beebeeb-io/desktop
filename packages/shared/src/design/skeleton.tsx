// ─── Skeleton loading primitives ────────────────────────────────
// Used in place of spinners / blank space while data loads.
// Style: bg-paper-2 animate-pulse rounded — consistent pulse animation.

/** A single text-like line skeleton. */
export function SkeletonLine({ width = '100%' }: { width?: string }) {
  return (
    <div
      className="h-3 bg-paper-2 animate-pulse rounded"
      style={{ width }}
    />
  )
}

/** A rectangular block skeleton — useful for icons, thumbnails, cards. */
export function SkeletonRect({ width, height }: { width: string; height: string }) {
  return (
    <div
      className="bg-paper-2 animate-pulse rounded"
      style={{ width, height }}
    />
  )
}

/** Mimics a file list row in the drive view. Matches the grid layout of drive.tsx. */
export function FileRowSkeleton() {
  return (
    <div
      className="animate-pulse"
      style={{
        display: 'grid',
        gridTemplateColumns: '20px 32px 1fr 110px 110px 100px 60px',
        gap: 14,
        padding: '11px 20px',
      }}
    >
      {/* Checkbox placeholder */}
      <span />
      {/* Icon */}
      <SkeletonRect width="28px" height="28px" />
      {/* Name + subtitle */}
      <div className="flex flex-col gap-1.5 justify-center">
        <SkeletonLine width="60%" />
        <SkeletonLine width="30%" />
      </div>
      {/* Size */}
      <div className="self-center">
        <SkeletonLine width="50px" />
      </div>
      {/* Modified */}
      <div className="self-center">
        <SkeletonLine width="56px" />
      </div>
      {/* Shared */}
      <div className="self-center">
        <SkeletonLine width="24px" />
      </div>
      {/* Actions */}
      <span />
    </div>
  )
}

/** Mimics a shared file row. Matches the grid layout of shared.tsx tabs. */
export function SharedRowSkeleton() {
  return (
    <div
      className="animate-pulse"
      style={{
        display: 'grid',
        gridTemplateColumns: '20px 32px 1.4fr 1fr 100px 80px',
        gap: 14,
        padding: '11px 18px',
      }}
    >
      {/* Checkbox placeholder */}
      <span />
      {/* Icon */}
      <SkeletonRect width="28px" height="28px" />
      {/* Name */}
      <div className="flex flex-col gap-1.5 justify-center">
        <SkeletonLine width="55%" />
      </div>
      {/* From / Shared with */}
      <div className="self-center">
        <SkeletonLine width="70%" />
      </div>
      {/* Size / Date */}
      <div className="self-center">
        <SkeletonLine width="50px" />
      </div>
      {/* Actions */}
      <span />
    </div>
  )
}

/** Mimics a stats/info card. */
export function CardSkeleton() {
  return (
    <div className="animate-pulse rounded-lg border border-line bg-paper p-4 flex flex-col gap-3">
      <SkeletonLine width="40%" />
      <SkeletonRect width="100%" height="32px" />
      <SkeletonLine width="60%" />
    </div>
  )
}

/** Mimics a trash row. Matches the grid layout of trash.tsx. */
export function TrashRowSkeleton() {
  return (
    <div
      className="animate-pulse"
      style={{
        display: 'grid',
        gridTemplateColumns: '24px 1.4fr 1fr 110px 110px 110px 100px',
        gap: 14,
        padding: '10px 20px',
      }}
    >
      <span />
      <div className="flex items-center gap-2.5">
        <SkeletonRect width="28px" height="28px" />
        <SkeletonLine width="55%" />
      </div>
      <div className="self-center"><SkeletonLine width="60%" /></div>
      <div className="self-center"><SkeletonLine width="56px" /></div>
      <div className="self-center"><SkeletonLine width="46px" /></div>
      <div className="self-center"><SkeletonLine width="50px" /></div>
      <span />
    </div>
  )
}

/** Mimics a photo grid tile. */
export function PhotoTileSkeleton() {
  return (
    <div
      className="animate-pulse rounded-md bg-paper-2 aspect-square"
      style={{ width: '100%' }}
    />
  )
}

/** Mimics a date-grouped photo grid (one heading + 8 tiles). */
export function PhotoGroupSkeleton() {
  return (
    <div className="mb-6">
      <div className="flex items-baseline mb-2.5 gap-2.5">
        <SkeletonLine width="80px" />
        <SkeletonLine width="60px" />
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-2">
        {Array.from({ length: 8 }, (_, i) => <PhotoTileSkeleton key={i} />)}
      </div>
    </div>
  )
}
