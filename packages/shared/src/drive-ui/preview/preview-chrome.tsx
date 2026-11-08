// The shared preview chrome: title bar, actions, and close.
import type { ReactNode } from 'react'
import { Icon } from '../../design'

interface PreviewChromeProps {
  filename: string
  kind: string
  size: string
  onClose: () => void
  children: ReactNode
  rightRail?: ReactNode
  /** True once decryption finished — drives the trust line below the title. */
  decrypted?: boolean
  /** Optional content rendered between the top bar and the body (e.g. version scrubber). */
  belowTopBar?: ReactNode
  /** Action callbacks — wired from the parent file preview */
  onDownload?: () => void
  onShare?: () => void
  onStar?: () => void
  isStarred?: boolean
  /** Image-specific controls */
  onZoomIn?: () => void
  onZoomOut?: () => void
  onRotate?: () => void
  /** Text/markdown/code editor entry point (task 1563). Only passed when the
   *  file is editable and not already in edit mode — see file-preview.tsx. */
  onEdit?: () => void
  /** True while the task-1563 editor is active. Switches the top bar into a
   *  single condensed row — breadcrumb/name + unsaved dot on the left, the
   *  editor's own controls (portaled in via `onToolbarSlotReady`) on the
   *  right — instead of stacking a second header underneath (PR #103
   *  review: "double header" — the filename used to appear in both this bar
   *  AND the editor's own header row). `onClose` still fires the same
   *  callback the caller wired in; it always closes the whole preview, in
   *  either mode — the editor's own "Done" (in its portaled toolbar) is the
   *  separate "exit edit mode, stay open" action. */
  editing?: boolean
  /** Unsaved-changes indicator, shown next to the filename while `editing`. */
  dirty?: boolean
  /** Ref callback for the DOM node the editor's own toolbar (Split preview /
   *  Wrap / Save) portals its buttons into — see file-editor.tsx. Only
   *  rendered while `editing`. */
  onToolbarSlotReady?: (el: HTMLDivElement | null) => void
}

export function PreviewChrome({
  filename,
  kind,
  size,
  onClose,
  children,
  rightRail,
  decrypted = false,
  belowTopBar,
  onDownload,
  onShare,
  onStar,
  isStarred = false,
  onZoomIn,
  onZoomOut,
  onRotate,
  onEdit,
  editing = false,
  dirty = false,
  onToolbarSlotReady,
}: PreviewChromeProps) {
  const kindIcon = kind.startsWith('image') ? 'file' : 'file' as const

  return (
    <div className="absolute inset-0 z-30 flex flex-col bg-paper text-ink">
      {/* Top bar */}
      <div className="flex shrink-0 items-center gap-2.5 border-b border-line bg-paper px-3 py-2">
        {/* Back button */}
        <button
          type="button"
          onClick={onClose}
          aria-label="Close preview"
          data-testid="preview-close-button"
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-paper-2 text-ink transition-colors hover:bg-paper-3"
        >
          <Icon name="chevron-right" size={13} className="rotate-180" />
        </button>

        {/* File info */}
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="inline-flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-[4px] bg-paper-2">
              <Icon name={kindIcon} size={11} />
            </span>
            <span className="min-w-0 truncate text-[13px] font-medium text-ink">
              {filename}
            </span>
            {editing ? (
              dirty && (
                <span
                  className="inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-amber-deep align-middle"
                  title="Unsaved changes"
                  data-testid="editor-dirty-dot"
                />
              )
            ) : (
              decrypted && (
                <span
                  className="inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-amber-deep"
                  title="Decrypted locally — key never left this device"
                />
              )
            )}
          </div>
          {/* Trust line — only on desktop where there's room, and not while
              editing (no room in the condensed single-row editor header). */}
          {decrypted && !editing && (
            <div
              key={filename}
              className="decrypt-fade-in hidden font-mono text-[10.5px] text-ink-3 mt-0.5 ml-[27px] items-center gap-1.5 sm:flex"
            >
              <span>Decrypted locally · key never left this device</span>
            </div>
          )}
        </div>

        {/* Editing: a single condensed row — the editor's own toolbar
            (Split preview / Wrap / Save) portals into this slot instead of
            a second header underneath. */}
        {editing && (
          <div
            ref={onToolbarSlotReady}
            className="flex shrink-0 items-center gap-1.5"
            data-testid="editor-toolbar-slot"
          />
        )}

        {/* Desktop action buttons — hidden on mobile (shown in bottom bar instead), and hidden entirely while editing (the editor's own toolbar takes over, above). */}
        {!editing && (
          <div className="hidden items-center gap-1.5 sm:flex">
            <span className="font-mono text-[11px] text-ink-3 mr-1">{size}</span>
            {onZoomOut && (
              <button
                type="button"
                onClick={onZoomOut}
                className="flex h-7 w-7 items-center justify-center rounded-md bg-paper-2 text-ink-2 transition-colors hover:bg-paper-3"
                aria-label="Zoom out"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="8" y1="11" x2="14" y2="11"/>
                </svg>
              </button>
            )}
            {onZoomIn && (
              <button
                type="button"
                onClick={onZoomIn}
                className="flex h-7 w-7 items-center justify-center rounded-md bg-paper-2 text-ink-2 transition-colors hover:bg-paper-3"
                aria-label="Zoom in"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="11" y1="8" x2="11" y2="14"/><line x1="8" y1="11" x2="14" y2="11"/>
                </svg>
              </button>
            )}
            {onRotate && (
              <button
                type="button"
                onClick={onRotate}
                className="flex h-7 w-7 items-center justify-center rounded-md bg-paper-2 text-ink-2 transition-colors hover:bg-paper-3"
                aria-label="Rotate clockwise"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38"/>
                </svg>
              </button>
            )}
            {onEdit && (
              <button
                type="button"
                onClick={onEdit}
                aria-label="Edit"
                aria-keyshortcuts="Meta+E"
                data-testid="preview-edit-button"
                className="flex h-7 items-center gap-1.5 rounded-md bg-amber px-2.5 text-[12px] font-medium text-[oklch(0.22_0.01_70)] transition-colors hover:brightness-95"
              >
                <Icon name="edit" size={12} />
                Edit
                <span className="font-mono text-[10px] opacity-70">⌘E</span>
              </button>
            )}
            {onShare && (
              <button
                type="button"
                onClick={onShare}
                className="flex h-7 w-7 items-center justify-center rounded-md bg-paper-2 text-ink-2 transition-colors hover:bg-paper-3"
                aria-label="Share"
              >
                <Icon name="share" size={13} />
              </button>
            )}
            {onDownload && (
              <button
                type="button"
                onClick={onDownload}
                className="flex h-7 w-7 items-center justify-center rounded-md bg-paper-2 text-ink-2 transition-colors hover:bg-paper-3"
                aria-label="Download"
              >
                <Icon name="download" size={13} />
              </button>
            )}
            {onStar && (
              <button
                type="button"
                onClick={onStar}
                className={`flex h-7 w-7 items-center justify-center rounded-md transition-colors ${
                  isStarred
                    ? 'bg-amber-bg text-amber-deep hover:bg-amber-bg/80'
                    : 'bg-paper-2 text-ink-2 hover:bg-paper-3'
                }`}
                aria-label={isStarred ? 'Unstar' : 'Star'}
              >
                <Icon name="star" size={13} />
              </button>
            )}
          </div>
        )}
      </div>

      {belowTopBar}

      {/* Body — stacked on mobile (canvas on top, info below), side-by-side on md+ */}
      <div className="flex min-h-0 flex-1 flex-col md:flex-row overflow-auto md:overflow-hidden">
        {/* Canvas area */}
        <div className="relative flex w-full md:flex-1 items-center justify-center bg-paper-2 p-4 md:p-6 min-h-[45vh] md:min-h-0">
          {children}
        </div>

        {/* Right rail — desktop only */}
        {rightRail && (
          <div className="w-full md:w-[280px] shrink-0 overflow-auto border-t md:border-t-0 md:border-l border-line bg-paper-2 p-[18px] text-ink-2">
            {rightRail}
          </div>
        )}
      </div>

      {/* Mobile action bar — shown only on small screens, below the image.
          Hidden while editing: the editor doesn't have a mobile-specific
          toolbar yet, and showing stale read-mode actions (Edit/Share/
          Download/Star) here would be confusing mid-edit. */}
      {!editing && (onDownload || onShare || onStar || onEdit) && (
        <div className="flex shrink-0 items-center justify-around border-t border-line bg-paper px-4 py-2.5 sm:hidden">
          {onEdit && (
            <button
              type="button"
              onClick={onEdit}
              className="flex flex-col items-center gap-1 text-amber-deep active:text-ink"
              aria-label="Edit"
            >
              <Icon name="edit" size={20} />
              <span className="text-[10px]">Edit</span>
            </button>
          )}
          {onShare && (
            <button
              type="button"
              onClick={onShare}
              className="flex flex-col items-center gap-1 text-ink-2 active:text-ink"
              aria-label="Share"
            >
              <Icon name="share" size={20} />
              <span className="text-[10px]">Share</span>
            </button>
          )}
          {onDownload && (
            <button
              type="button"
              onClick={onDownload}
              className="flex flex-col items-center gap-1 text-ink-2 active:text-ink"
              aria-label="Download"
            >
              <Icon name="download" size={20} />
              <span className="text-[10px]">Download</span>
            </button>
          )}
          {onStar && (
            <button
              type="button"
              onClick={onStar}
              className={`flex flex-col items-center gap-1 ${isStarred ? 'text-amber-deep' : 'text-ink-2'} active:text-ink`}
              aria-label={isStarred ? 'Unstar' : 'Star'}
            >
              <Icon name="star" size={20} />
              <span className="text-[10px]">{isStarred ? 'Starred' : 'Star'}</span>
            </button>
          )}
        </div>
      )}
    </div>
  )
}
