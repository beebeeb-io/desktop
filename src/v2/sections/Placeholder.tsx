// A section no batch has built yet: says so plainly, never pretends to be the finished frame.
export function Placeholder({ title, batch, frames }: { title: string; batch: string; frames: string }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
      <h1 className="text-[15px] font-semibold text-ink">{title}</h1>
      <p className="max-w-sm text-[13px] text-ink-3">
        Not built yet: batch {batch} builds this section ({frames}).
      </p>
    </div>
  )
}
