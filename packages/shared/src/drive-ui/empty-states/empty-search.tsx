// The empty state for search with no matches.
import { Icon } from '../../design'
import { BBButton } from '../../design'

interface EmptySearchProps {
  query: string
  onClearSearch: () => void
  onCheckTrash?: () => void
  onSearchShared?: () => void
}

export function EmptySearch({
  query,
  onClearSearch,
  onCheckTrash,
  onSearchShared,
}: EmptySearchProps) {
  return (
    <div className="flex-1 flex items-center justify-center py-12">
      <div className="max-w-[420px] text-center">
        {/* Search icon */}
        <div className="w-10 h-10 mx-auto mb-3.5 rounded-xl flex items-center justify-center bg-paper-2 border border-line text-ink-4">
          <Icon name="search" size={18} />
        </div>

        <h3 className="text-[15px] font-semibold text-ink mb-1.5">
          No files found for &ldquo;{query}&rdquo;
        </h3>

        <p className="text-[12.5px] text-ink-3 leading-relaxed mb-2 max-w-[400px] mx-auto">
          We search filenames on your device — file contents stay encrypted
          on our servers. If it was in a shared folder, ask the owner to
          re-share.
        </p>

        <p className="text-[11.5px] text-ink-4 leading-relaxed mb-4 max-w-[360px] mx-auto">
          Tip: Try searching for a file type like &ldquo;pdf&rdquo;, &ldquo;jpg&rdquo;, or a date like &ldquo;May&rdquo;
        </p>

        <div className="flex gap-1.5 justify-center flex-wrap">
          {onCheckTrash && (
            <BBButton size="sm" onClick={onCheckTrash} className="gap-1.5">
              <Icon name="clock" size={11} /> Check Trash
            </BBButton>
          )}
          {onSearchShared && (
            <BBButton size="sm" onClick={onSearchShared} className="gap-1.5">
              <Icon name="users" size={11} /> Search shared with me
            </BBButton>
          )}
          <BBButton size="sm" variant="amber" onClick={onClearSearch} className="gap-1.5">
            <Icon name="x" size={11} /> Clear search
          </BBButton>
        </div>
      </div>
    </div>
  )
}
