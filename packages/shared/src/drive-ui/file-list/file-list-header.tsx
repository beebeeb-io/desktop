// The file-list header row with sort and selection controls.
import { BBCheckbox } from '../../design'
import { Icon } from '../icons'
import { FILE_HEADER_BAR, FILE_HEADER_LABEL } from '../file-browser-row'
import { SORT_OPTIONS, type SortKey } from '../file-list-sort'

/**
 * The sort button and the column header row of the drive file list (W-P2-4).
 * Sort state stays in `FileList`; every control here only reports its intent.
 */
export interface FileListHeaderProps {
  sortKey: SortKey
  sortMenuOpen: boolean
  onToggleSortMenu(): void
  onCloseSortMenu(): void
  onSelectSort(key: SortKey): void
  onToggleNameSort(): void
  onToggleSizeSort(): void
  onToggleModifiedSort(): void
  selectable: boolean
  totalCount: number
  allSelected: boolean
  someSelected: boolean
  onToggleAll(): void
}

export function FileListHeader({
  sortKey,
  sortMenuOpen,
  onToggleSortMenu,
  onCloseSortMenu,
  onSelectSort,
  onToggleNameSort,
  onToggleSizeSort,
  onToggleModifiedSort,
  selectable,
  totalCount,
  allSelected,
  someSelected,
  onToggleAll,
}: FileListHeaderProps) {
  const sortLabel = SORT_OPTIONS.find((o) => o.key === sortKey)?.label ?? 'Sort'

  return (
    <>
      <div className="px-3 md:px-5 py-1.5 border-b border-line bg-paper-2 flex items-center justify-end relative">
        <button
          type="button"
          onClick={onToggleSortMenu}
          className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-ink-2 rounded hover:bg-paper hover:text-ink transition-colors"
          aria-label={`Sort: ${sortLabel}`}
          aria-haspopup="menu"
          aria-expanded={sortMenuOpen}
        >
          <span className="text-ink-3">Sort:</span>
          <span className="font-medium">{sortLabel}</span>
          <Icon name="chevron-down" size={10} className="text-ink-3" />
        </button>
        {sortMenuOpen && (
          <>
            <div className="fixed inset-0 z-30" onClick={onCloseSortMenu} />
            <div
              role="menu"
              className="absolute right-3 md:right-5 top-full mt-0.5 z-40 bg-paper border border-line-2 rounded-md shadow-2 py-1 min-w-[200px]"
            >
              {SORT_OPTIONS.map((opt) => (
                <button
                  key={opt.key}
                  role="menuitemradio"
                  aria-checked={opt.key === sortKey}
                  onClick={() => onSelectSort(opt.key)}
                  className={`w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left hover:bg-paper-2 transition-colors ${
                    opt.key === sortKey ? 'text-ink font-medium' : 'text-ink-2'
                  }`}
                >
                  <span className="w-3 inline-flex">
                    {opt.key === sortKey && <Icon name="check" size={10} />}
                  </span>
                  {opt.label}
                </button>
              ))}
            </div>
          </>
        )}
      </div>

      <div className={`${FILE_HEADER_BAR} grid-cols-[20px_32px_1fr_40px] md:grid-cols-[20px_32px_1fr_110px_110px_100px_60px]`}>
        <span className="flex items-center justify-center">
          {selectable && totalCount > 0 && (
            <BBCheckbox
              checked={allSelected}
              indeterminate={someSelected}
              onChange={onToggleAll}
            />
          )}
        </span>
        <span />
        {/* Name header — toggles name-asc / name-desc */}
        <button
          type="button"
          onClick={onToggleNameSort}
          className={`flex items-center gap-1 ${FILE_HEADER_LABEL} hover:text-ink transition-colors cursor-pointer select-none`}
          aria-label="Sort by name"
        >
          Name
          <span className="text-[9px] leading-none">
            {sortKey === 'name-asc' ? '↑' : sortKey === 'name-desc' ? '↓' : <span className="opacity-30">↕</span>}
          </span>
        </button>
        {/* Size header — toggles size-desc / size-asc */}
        <button
          type="button"
          onClick={onToggleSizeSort}
          className={`hidden md:flex items-center gap-1 ${FILE_HEADER_LABEL} hover:text-ink transition-colors cursor-pointer select-none`}
          aria-label="Sort by size"
        >
          Size
          <span className="text-[9px] leading-none">
            {sortKey === 'size-desc' ? '↓' : sortKey === 'size-asc' ? '↑' : <span className="opacity-30">↕</span>}
          </span>
        </button>
        {/* Modified header — toggles modified-desc / modified-asc */}
        <button
          type="button"
          onClick={onToggleModifiedSort}
          className={`hidden md:flex items-center gap-1 ${FILE_HEADER_LABEL} hover:text-ink transition-colors cursor-pointer select-none`}
          aria-label="Sort by modified date"
        >
          Modified
          <span className="text-[9px] leading-none">
            {sortKey === 'modified-desc' ? '↓' : sortKey === 'modified-asc' ? '↑' : <span className="opacity-30">↕</span>}
          </span>
        </button>
        <span className={`hidden md:inline ${FILE_HEADER_LABEL}`}>Shared</span>
        <span />
      </div>
    </>
  )
}
