// One file-list row: name, metadata, and action handlers.
import type { ReactNode, RefObject } from 'react'
import { BBCheckbox } from '../../design'
import { BBChip } from '../../design'
import { BBButton } from '../../design'
import { Icon } from '../icons'
import { FileIcon, getFileType } from '../file-icon'
import { getBadgeStyle } from '../lib/file-colors'
import { FileBrowserRow } from '../file-browser-row'
import { FolderViewerBadge } from '../folder-viewer-badge'
import { formatBytes, timeAgo } from '../lib/format'
import { fileStorageCountry } from '../lib/storage-country'
import type { DriveFile } from '../../types'

/**
 * One row of the drive file list (W-P2-4). Purely presentational: every
 * decision and side effect stays in `FileList` and reaches the row through
 * `flags`, `thumbnail`, `inlineRename` and the `handlers` closures.
 */
export interface FileRowProps {
  file: DriveFile
  /** The file's index into `sortedFiles`; keyboard navigation needs it. */
  rowIndex: number
  /** Decrypted name, or null while the name is still being decrypted. */
  name: string | null
  selection: {
    selectable: boolean
    isSelected: boolean
    /** This row is the one opened in the details panel. */
    isCurrent: boolean
    /** Any row at all is selected (drives the drag-grip visibility). */
    anySelected: boolean
  }
  flags: {
    isDragTarget: boolean
    isDragging: boolean
    isTrashing: boolean
    isRecentUpload: boolean
    isPinned: boolean
    isStarPulse: boolean
    isDecryptPulse: boolean
    isUnlocked: boolean
    /** Share badge: a live share exists (`share_count`, or the myShares fallback). */
    hasActiveShare: boolean
    /** The viewer gets the trust-details button (unlocked, non-folder, handler present). */
    canShowTrustDetails: boolean
    /**
     * This row's id is of the foreign class (core's answer). Set from the id
     * class, never from `uploaded_by`/`acted_via_share_id` (spec 2275).
     */
    isForeign: boolean
  }
  thumbnail: {
    /** A thumbnail exists and the vault is unlocked. */
    enabled: boolean
    url: string | undefined
    /** Folder label dot colour, null when the folder has no label. */
    folderColor: string | null
    viewerCount: { count: number; emails: string[] } | undefined
  }
  inlineRename: {
    editing: boolean
    value: string
    inputRef: RefObject<HTMLInputElement | null>
    onChange(value: string): void
    commit(): void
    cancel(): void
  }
  /** Replaces the kebab button when set (e.g. the Shared page's own menu). */
  renderActions?: (file: DriveFile) => ReactNode
  handlers: FileRowHandlers
}

export interface FileRowHandlers {
  onDragStart(e: React.DragEvent): void
  onDragEnd(): void
  onDragOver(e: React.DragEvent): void
  onDragLeave(e: React.DragEvent): void
  onDrop(e: React.DragEvent): void
  /** Capture-phase clear of a stale debounced select (see pending-select-timer). */
  onInteractionCapture(): void
  onClick(e: React.MouseEvent): void
  onDoubleClick(e: React.MouseEvent): void
  onContextMenu(e: React.MouseEvent): void
  onKeyDown(e: React.KeyboardEvent): void
  onCheckboxClick(e: React.MouseEvent): void
  onToggleStar(e: React.MouseEvent): void
  onShowTrustDetails(e: React.MouseEvent): void
  /** Opens the share popover, anchored at the clicked badge. */
  onOpenShare(e: React.MouseEvent): void
  /** Opens the row context menu, anchored at the clicked element. */
  onOpenMenu(e: React.MouseEvent): void
  setThumbRef(el: HTMLElement | null): void
}

export function FileRow({
  file,
  rowIndex,
  name,
  selection,
  flags,
  thumbnail,
  inlineRename,
  renderActions,
  handlers,
}: FileRowProps) {
  const fileType = getFileType(name ?? '', file.is_folder)
  const { isSelected, selectable, anySelected } = selection
  const shareCount = file.share_count ?? 0
  const versionNumber = file.version_number ?? 1

  return (
    <FileBrowserRow
      key={file.id}
      data-row-index={rowIndex}
      draggable={!flags.isTrashing}
      tabIndex={0}
      role="row"
      aria-label={
        file.is_folder
          ? `${name ?? 'Folder'}, folder`
          : `${name ?? 'Encrypted file'}, ${formatBytes(file.size_bytes)}, ${timeAgo(file.updated_at)}`
      }
      aria-selected={isSelected || undefined}
      onDragStart={handlers.onDragStart}
      onDragEnd={handlers.onDragEnd}
      onDragOver={handlers.onDragOver}
      onDragLeave={handlers.onDragLeave}
      onDrop={handlers.onDrop}
      className={[
        'grid-cols-[20px_32px_1fr_40px] md:grid-cols-[20px_32px_1fr_110px_110px_100px_60px]',
        'px-3 md:px-5 py-[11px]',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-amber',
        flags.isDragTarget
          ? 'file-row--drag-target bg-amber-bg ring-2 ring-amber ring-inset'
          : isSelected
            ? 'file-row--selected bg-amber-bg/50'
            : selection.isCurrent
              ? 'bg-amber-bg/30'
              : 'hover:bg-paper-2',
        flags.isDragging ? 'opacity-40' : '',
        flags.isTrashing ? 'trash-slide-out' : '',
        flags.isRecentUpload ? 'upload-glow' : '',
      ].filter(Boolean).join(' ')}
      // Capture phase: runs before ANY nested control's onClick (checkbox,
      // star, share badges, lock, row actions, rename input — all of which
      // stopPropagation and so never reach the row's onClick), clearing a
      // debounced select-timer armed by an earlier non-previewable click.
      // See handleRowInteractionCapture in pending-select-timer.ts.
      onClickCapture={handlers.onInteractionCapture}
      onClick={handlers.onClick}
      onDoubleClick={handlers.onDoubleClick}
      onContextMenu={handlers.onContextMenu}
      onKeyDown={handlers.onKeyDown}
    >
      {/* Checkbox — shows on hover (or always when selected); drag grip shows on hover when nothing is selected */}
      <span className="relative flex items-center justify-center">
        {/* Drag grip — shows behind the checkbox when hovering and no selection is active */}
        <span
          className={`absolute inset-0 flex items-center justify-center transition-opacity text-ink-4 cursor-grab active:cursor-grabbing pointer-events-none ${
            anySelected ? 'opacity-0' : 'opacity-0 group-hover:opacity-100'
          }`}
          aria-hidden="true"
        >
          <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
            <circle cx="3" cy="2" r="1.2" />
            <circle cx="7" cy="2" r="1.2" />
            <circle cx="3" cy="7" r="1.2" />
            <circle cx="7" cy="7" r="1.2" />
            <circle cx="3" cy="12" r="1.2" />
            <circle cx="7" cy="12" r="1.2" />
          </svg>
        </span>
        {/* Checkbox — on top of the grip; visible when selected or when items are already selected */}
        <span
          className={`relative z-10 transition-opacity ${
            selectable && isSelected ? '' : 'opacity-0 group-hover:opacity-100'
          }`}
          onClick={handlers.onCheckboxClick}
        >
          {selectable && <BBCheckbox checked={isSelected} onChange={() => {}} />}
        </span>
      </span>

      {/* Icon or thumbnail */}
      {thumbnail.enabled ? (
        <div
          ref={(el) => handlers.setThumbRef(el as HTMLElement | null)}
          data-file-id={file.id}
          className="w-8 h-8 rounded-md overflow-hidden shrink-0 self-center"
        >
          {thumbnail.url ? (
            <img src={thumbnail.url} alt="" className="w-full h-full object-cover" />
          ) : (
            <div className="w-full h-full bg-paper-3 flex items-center justify-center">
              <FileIcon type={fileType} />
            </div>
          )}
        </div>
      ) : (
        <div className="relative inline-flex shrink-0">
          <FileIcon type={fileType} />
          {file.is_folder && thumbnail.folderColor && (
            <span
              aria-hidden="true"
              style={{
                position: 'absolute',
                bottom: 0,
                right: 0,
                width: 8,
                height: 8,
                borderRadius: '50%',
                backgroundColor: thumbnail.folderColor,
                boxShadow: '0 0 0 1.5px var(--color-paper)',
              }}
            />
          )}
        </div>
      )}

      {/* Name column */}
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          {inlineRename.editing ? (
            <input
              ref={inlineRename.inputRef}
              className="font-medium bg-transparent border border-amber-deep rounded px-1 outline-none min-w-0 flex-1"
              value={inlineRename.value}
              onChange={(e) => inlineRename.onChange(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') { e.preventDefault(); inlineRename.commit() }
                if (e.key === 'Escape') { e.preventDefault(); inlineRename.cancel() }
              }}
              onBlur={inlineRename.commit}
              onClick={(e) => e.stopPropagation()}
            />
          ) : name !== null
            ? <span className="font-medium truncate">{name}</span>
            : <span
                className="inline-block rounded bg-paper-3 animate-pulse shrink-0"
                style={{ width: `${Math.max(60, (file.id.charCodeAt(0) % 80) + 60)}px`, height: '12px' }}
                aria-label="Loading…"
              />
          }
          {/* F row marker — from the id class only, never a member or link label. */}
          {flags.isForeign && (
            <span
              data-testid="foreign-row-marker"
              className="inline-flex items-center shrink-0 rounded border border-line bg-paper-2 px-1 py-px font-mono text-[9px] text-ink-3"
              title="Added by someone else"
            >
              Added by someone else
            </span>
          )}
          {/* Star toggle — filled when starred, outline when not */}
          <button
            type="button"
            onClick={handlers.onToggleStar}
            aria-label={file.is_starred ? 'Unstar' : 'Star'}
            aria-pressed={!!file.is_starred}
            className={`inline-flex items-center transition-colors cursor-pointer focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-amber rounded ${
              file.is_starred
                ? 'text-amber-deep'
                : 'text-ink-4 opacity-0 group-hover:opacity-100 hover:text-amber-deep'
            }${flags.isStarPulse ? ' star-pulse' : ''}`}
          >
            <Icon name="star" size={11} />
          </button>

          {/* Pin badge — shown when folder is pinned to Quick access */}
          {flags.isPinned && (
            <span
              className="inline-flex items-center text-amber-deep"
              title="Pinned to Quick access"
              aria-label="Pinned to Quick access"
            >
              <Icon name="pin" size={10} />
            </span>
          )}

          {/* Folder viewer badge — compact presence dots for shared folders */}
          {file.is_folder && thumbnail.viewerCount && thumbnail.viewerCount.count > 0 && (
            <FolderViewerBadge
              count={thumbnail.viewerCount.count}
              emails={thumbnail.viewerCount.emails}
            />
          )}

          {/* Share badge — shown when file has active share links.
              `FileList` folds stale `share_count` into `hasActiveShare` (the
              active myShares fallback), so this only reads the badge inputs. */}
          {flags.hasActiveShare && (
            <button
              type="button"
              onClick={handlers.onOpenShare}
              aria-label={
                shareCount > 0
                  ? `Manage ${shareCount} active share${shareCount > 1 ? 's' : ''}`
                  : 'Manage active share'
              }
              title="Shared"
              className="inline-flex items-center gap-0.5 text-amber-deep hover:text-amber transition-colors cursor-pointer"
            >
              <Icon name="link" size={11} />
              {shareCount > 1 && (
                <span className="font-mono text-[9px]">{shareCount}</span>
              )}
            </button>
          )}
          {flags.isUnlocked && (
            flags.canShowTrustDetails ? (
              <button
                type="button"
                onClick={handlers.onShowTrustDetails}
                aria-label="Encryption details"
                className={`inline-flex items-center justify-center w-[18px] h-[18px] rounded-full text-amber-deep hover:bg-amber-bg transition-colors cursor-pointer ${flags.isDecryptPulse ? 'decrypt-pulse' : ''}`}
                title="View encryption details"
              >
                <Icon name="lock" size={11} />
              </button>
            ) : (
              <span className={flags.isDecryptPulse ? 'decrypt-pulse inline-flex' : 'inline-flex'}>
                <BBChip variant="amber">
                  <span className="flex items-center gap-1 text-[9.5px]">
                    <Icon name="lock" size={9} /> E2EE
                  </span>
                </BBChip>
              </span>
            )
          )}
          {!file.is_folder && versionNumber > 1 && (
            <span
              className="font-mono text-[9.5px] text-ink-3 bg-paper-3 border border-line rounded px-1 py-px tabular-nums"
              title={`Version ${file.version_number}`}
            >
              v{file.version_number}
            </span>
          )}
          {file.is_trashed && (
            <BBChip variant="default" className="gap-1 py-0 text-[9.5px]">
              <Icon name="trash" size={9} /> Deleted
            </BBChip>
          )}
        </div>
        <div className="text-[11px] text-ink-3 mt-0.5 flex items-center gap-1.5 flex-wrap">
          {!file.is_folder && name !== null ? (() => {
            const ext = name.split('.').pop()?.toLowerCase() ?? ''
            const badgeStyle = getBadgeStyle(fileType)
            return ext ? (
              <span
                className="font-mono font-medium shrink-0 rounded px-[5px] py-px text-[10px] leading-tight"
                style={{ color: badgeStyle.color, background: badgeStyle.background }}
              >
                {ext.toUpperCase()}
              </span>
            ) : null
          })() : null}
          <span className="truncate">{typeLabel(file)}</span>
          {!file.is_folder && (
            <>
              <span className="md:hidden text-line-2">·</span>
              <span className="md:hidden font-mono tabular-nums">{formatBytes(file.size_bytes)}</span>
            </>
          )}
          <span className="md:hidden text-line-2">·</span>
          <span className="md:hidden">{timeAgo(file.updated_at)}</span>
        </div>
        {!file.is_folder && (
          <div className="font-mono text-[10.5px] text-ink-3 mt-0.5 truncate tabular-nums">
            AES-256-GCM · {fileStorageCountry(file) ?? 'Europe'}
          </div>
        )}
      </div>

      {/* Size */}
      <span className="hidden md:inline font-mono text-[13px] text-ink-3 tabular-nums self-center">
        {file.is_folder ? '--' : formatBytes(file.size_bytes)}
      </span>

      {/* Modified */}
      <span className="hidden md:inline text-[13px] text-ink-3 self-center">
        {timeAgo(file.updated_at)}
      </span>

      {/* Shared indicator */}
      <div className="hidden md:block self-center">
        {shareCount > 0 ? (
          <button
            type="button"
            onClick={handlers.onOpenShare}
            aria-label={`${shareCount} active share link${shareCount > 1 ? 's' : ''}`}
            className="inline-flex items-center gap-1 text-[12px] text-amber-deep hover:text-amber transition-colors cursor-pointer font-medium"
          >
            <Icon name="link" size={12} />
            {shareCount > 1 && <span className="font-mono text-[11px]">{shareCount}</span>}
          </button>
        ) : (
          <span className="text-[13px] text-ink-4">—</span>
        )}
      </div>

      {/* Row actions */}
      <div className="flex justify-end self-center">
        {renderActions ? (
          renderActions(file)
        ) : (
          <BBButton
            size="sm"
            variant="ghost"
            className="md:opacity-0 md:group-hover:opacity-100 transition-opacity"
            aria-label="File actions"
            onClick={handlers.onOpenMenu}
          >
            <Icon name="more" size={14} />
          </BBButton>
        )}
      </div>
    </FileBrowserRow>
  )
}

function typeLabel(file: DriveFile): string {
  if (file.is_folder) return 'Folder'
  if (file.mime_type) return file.mime_type
  // Extension is already shown in the colored badge — don't repeat it
  return ''
}
