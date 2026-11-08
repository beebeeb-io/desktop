// DW-1105: instant search over the list already on this PC. A view: the host passes the query and the results.
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType, type DriveItem } from '@beebeeb/shared/drive-ui'
import { FOCUS } from './ui'
import { fmtSize, fmtWhen, middleShorten } from './model'

export interface SearchHit { item: DriveItem; where: string }
export const PILLS = ['All', 'Folders', 'Documents', 'Images', 'Videos'] as const
export type Pill = (typeof PILLS)[number]

export function SearchView({ query, hits, pill = 'All', onQuery, onPill, onOpen, onEnd }: {
  query: string; hits: SearchHit[]; pill?: Pill; onQuery?: (q: string) => void; onPill?: (p: Pill) => void; onOpen?: (item: DriveItem) => void; onEnd?: () => void
}) {
  return (
    <section aria-label="Search" data-search className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <Icon name="search" size={16} />
        <input ref={(el) => el?.focus({ preventScroll: true })} aria-label="Search names" value={query} onChange={(e) => onQuery?.(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') onEnd?.() }} className="h-8 min-w-0 flex-1 bg-transparent text-[14px] text-ink outline-none" />
        <button type="button" onClick={onEnd} className={`h-8 rounded-md px-3 text-[13px] text-ink-2 hover:bg-paper-2 ${FOCUS}`}>Esc</button>
      </div>
      <div role="group" aria-label="Narrow by type" className="flex flex-none gap-2 border-b border-line px-4 py-2">
        {PILLS.map((p) => (
          <button key={p} type="button" aria-pressed={pill === p} onClick={() => onPill?.(p)} className={`h-7 rounded-full border px-3 text-[12px] ${pill === p ? 'border-ink-3 bg-paper-3 text-ink' : 'border-line text-ink-2 hover:bg-paper-2'} ${FOCUS}`}>{p}</button>
        ))}
      </div>
      <div role="list" aria-label="Results" data-results={hits.length} className="min-h-0 flex-1 overflow-auto">
        {hits.map(({ item, where }) => (
          <div key={item.id} role="listitem" data-result={item.id} onDoubleClick={() => onOpen?.(item)} className="flex h-12 cursor-default items-center gap-3 px-4 text-[13px] hover:bg-paper-2">
            <FileIcon type={getFileType(item.name, item.isFolder)} size={24} />
            <span className="min-w-0 flex-1"><span className="block truncate text-ink" title={item.name}>{middleShorten(item.name)}</span><span className="block truncate text-[12px] text-ink-3">{where}</span></span>
            <span className="font-mono text-[12px] text-ink-3">{item.isFolder ? '' : fmtSize(item.size)}</span>
            <span className="w-32 text-right font-mono text-[12px] text-ink-3">{fmtWhen(item.modifiedAt, new Date('2026-11-08T12:00:00'))}</span>
          </div>
        ))}
      </div>
      <p className="m-0 flex-none border-t border-line bg-paper-2 px-4 py-2 text-[12px] leading-4 text-ink-3">Searches file and folder names on this PC, even offline. It doesn’t look inside files.</p>
    </section>
  )
}
