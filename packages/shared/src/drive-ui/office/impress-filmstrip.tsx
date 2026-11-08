/**
 * ImpressFilmstrip (task 1567, Impress lane) — left rail of slide thumbnails,
 * matching design/office-editor.html's `.filmstrip`/`.slidethumb` (196px
 * wide, 16:9 tiles, amber ring on the active one).
 *
 * Every button here drives a real, probe-verified `.uno:` command (see
 * `impress-commands.ts`'s header for the evidence) via `bbOffice.dispatch`,
 * never a local-only reorder — the filmstrip's own list is DERIVED from
 * `.uno:PageStatus`'s slide count (`slideCount`/`activeIndex` props, owned by
 * OfficeEditor's `useImpressChrome` hook), not tracked independently, so it
 * can never show a count the engine itself disagrees with.
 *
 * Each tile is a generic numbered schematic, not a content preview — no
 * bridge call exposes real per-slide text or a thumbnail bitmap (see
 * `impress-commands.ts`'s "ALSO NOT FOUND" note). Showing an invented
 * title/bullet layout per slide would misrepresent content this component
 * cannot actually read.
 */

import { canDelete, canMoveDown, canMoveUp, slideIndices } from '../lib/office/impress-commands'
import { IconChevronDown } from './office-icons'
import { Icon } from '../../design'

export interface ImpressFilmstripProps {
  /** Null while `.uno:PageStatus`'s first event hasn't arrived yet. */
  status: { index: number; count: number } | null
  onSelect: (index: number) => void
  onInsert: () => void
  onDuplicate: () => void
  onDelete: () => void
  onMoveUp: () => void
  onMoveDown: () => void
  busy: boolean
}

function SlideTile({ n, active, onSelect }: { n: number; active: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={active}
      aria-label={`Slide ${n}`}
      data-testid={`impress-slide-${n}`}
      className={`relative flex aspect-video shrink-0 flex-col justify-center gap-1 rounded-md border-2 bg-white px-2.5 py-2 shadow-2 ${
        active ? 'border-amber' : 'border-transparent'
      }`}
    >
      <span className="absolute left-1 top-0.5 font-mono text-[9px] font-semibold text-black/35">{n}</span>
      <i className="block h-[5px] w-[65%] rounded-sm bg-black/75" />
      <i className="block h-[3.5px] w-[85%] rounded-sm bg-black/40" />
      <i className="block h-[3.5px] w-[55%] rounded-sm bg-black/40" />
    </button>
  )
}

export function ImpressFilmstrip({ status, onSelect, onInsert, onDuplicate, onDelete, onMoveUp, onMoveDown, busy }: ImpressFilmstripProps) {
  const count = status?.count ?? 0
  const activeIndex = status?.index ?? 0
  const indices = slideIndices(count)

  return (
    <div className="flex w-[196px] shrink-0 flex-col border-r border-line bg-paper-2" data-testid="impress-filmstrip">
      <div className="flex items-center justify-between px-3.5 pb-2 pt-3.5">
        <span className="text-[10.5px] font-semibold uppercase tracking-wider text-ink-3">Slides</span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            aria-label="Move slide up"
            data-testid="impress-move-up"
            disabled={busy || !canMoveUp(activeIndex)}
            onClick={onMoveUp}
            className="grid h-[20px] w-[20px] place-items-center rounded text-ink-3 hover:bg-paper-3 disabled:opacity-30"
          >
            <IconChevronDown size={11} className="rotate-180" />
          </button>
          <button
            type="button"
            aria-label="Move slide down"
            data-testid="impress-move-down"
            disabled={busy || !canMoveDown(activeIndex, count)}
            onClick={onMoveDown}
            className="grid h-[20px] w-[20px] place-items-center rounded text-ink-3 hover:bg-paper-3 disabled:opacity-30"
          >
            <IconChevronDown size={11} />
          </button>
        </div>
      </div>
      <div className="flex flex-col gap-2.5 overflow-auto px-3 pb-3">
        {indices.length === 0 && <div className="px-1 py-1.5 text-[12px] text-ink-4">No slides yet</div>}
        {indices.map((n) => (
          <SlideTile key={n} n={n} active={n === activeIndex} onSelect={() => onSelect(n)} />
        ))}
      </div>
      <div className="mt-auto flex items-center gap-1.5 border-t border-line px-3 py-2.5">
        <button
          type="button"
          data-testid="impress-slide-insert"
          disabled={busy}
          onClick={onInsert}
          className="flex h-7 flex-1 items-center justify-center gap-1 rounded-md border border-line text-[12px] font-medium text-ink hover:bg-paper-3 disabled:opacity-40"
        >
          <Icon name="plus" size={11} />
          New
        </button>
        <button
          type="button"
          aria-label="Duplicate slide"
          data-testid="impress-slide-duplicate"
          disabled={busy || count < 1}
          onClick={onDuplicate}
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-line text-ink-3 hover:bg-paper-3 disabled:opacity-40"
          title="Duplicate slide"
        >
          <Icon name="copy" size={12} />
        </button>
        <button
          type="button"
          aria-label="Delete slide"
          data-testid="impress-slide-delete"
          disabled={busy || !canDelete(count)}
          onClick={onDelete}
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-line text-ink-3 hover:bg-red-bg hover:text-red disabled:opacity-40"
          title="Delete slide"
        >
          <Icon name="trash" size={12} />
        </button>
      </div>
    </div>
  )
}
