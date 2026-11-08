// The move dialog: pick a destination folder for the selected items.
import { useState, useEffect, useCallback, useRef } from 'react'
import { useFocusTrap } from './hooks/use-focus-trap'
import { BBButton, BBChip, Icon } from '../design'
import { isSharedRootById, pickerFolderLabel } from './lib/display-markers'

/** What the picker needs from its host: folder listing with names already decrypted, and folder creation. */
export interface MoveModalHost {
  /** The folders directly under `parentId` (null = root), names decrypted for display. */
  listFolders(parentId: string | null): Promise<{ id: string; name: string }[]>
  /** Creates a folder; throws on refusal. */
  newFolder(parentId: string | null, name: string): Promise<{ id: string; name: string }>
  /** The host shows its own notice for a failed create. */
  onCreateFailed(err: unknown): void
}

export interface MoveModalProps {
  host: MoveModalHost
  open: boolean
  onClose: () => void
  items: { id: string; name: string; isFolder: boolean }[]
  mode?: 'move' | 'copy'
  /** `pickerRootId` is the shared root the picker showed for the destination
   * (or null when it showed none): L15 (b)3's answer, and the only thing that
   * lets the move publish wraps. */
  onConfirm: (destinationId: string | null, mode: 'move' | 'copy', pickerRootId: string | null) => void
}

interface FolderNode {
  id: string
  name: string
  children: FolderNode[]
  loaded: boolean
  expanded: boolean
}

export function MoveModal({
  host,
  open,
  onClose,
  items,
  mode: initialMode = 'move',
  onConfirm,
}: MoveModalProps) {
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [breadcrumbs, setBreadcrumbs] = useState<{ id: string | null; name: string }[]>([
    { id: null, name: 'My files' },
  ])
  const [search, setSearch] = useState('')
  const [loading, setLoading] = useState(false)
  const [creatingFolder, setCreatingFolder] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const focusTrapRef = useFocusTrap<HTMLDivElement>(open)

  function displayName(folder: FolderNode): string {
    return folder.name
  }

  // Fetch folders for the current directory
  const fetchFolders = useCallback(async (parentId?: string) => {
    setLoading(true)
    try {
      const folderFiles = await host.listFolders(parentId ?? null)
      setFolders(
        folderFiles.map((f) => ({
          id: f.id,
          name: f.name,
          children: [],
          loaded: false,
          expanded: false,
        })),
      )
    } catch (err) {
      console.error('[MoveModal] Failed to load folders:', err)
      setFolders([])
    } finally {
      setLoading(false)
    }
  }, [host])

  useEffect(() => {
    if (open) {
      const currentParent = breadcrumbs[breadcrumbs.length - 1]?.id ?? undefined
      fetchFolders(currentParent)
      setSelectedId(null)
      setSearch('')
    }
  }, [open, breadcrumbs, fetchFolders])

  // Close on escape
  useEffect(() => {
    if (!open) return
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [open, onClose])

  const handleBreadcrumbNav = (index: number) => {
    setBreadcrumbs((prev) => prev.slice(0, index + 1))
  }

  const handleFolderOpen = (folder: FolderNode) => {
    setBreadcrumbs((prev) => [...prev, { id: folder.id, name: displayName(folder) }])
    setSelectedId(null)
  }

  const handleNewFolder = async () => {
    const name = search.trim() || 'Untitled folder'
    setCreatingFolder(true)
    try {
      const parentId = breadcrumbs[breadcrumbs.length - 1]?.id ?? undefined
      // The host seals the name and refuses while the vault is locked.
      await host.newFolder(parentId ?? null, name)
      setSearch('')
      fetchFolders(parentId)
    } catch (err) {
      // Shown, as on every other create path: a failed create (the changed-id
      // abort included) must not look like nothing happened.
      host.onCreateFailed(err)
      setSearch('')
    } finally {
      setCreatingFolder(false)
    }
  }

  const handleConfirm = (actionMode: 'move' | 'copy') => {
    const destinationId = selectedId ?? breadcrumbs[breadcrumbs.length - 1]?.id ?? null
    // L15 (b)3: the shared root this picker actually showed for the chosen
    // destination — the selected row or any folder on the current breadcrumb
    // trail. Its absence is an answer too: nothing is sealed under a guess.
    const pickerRootId =
      [selectedId, ...breadcrumbs.map((crumb) => crumb.id).reverse()].find(
        (id): id is string => !!id && isSharedRootById(id),
      ) ?? null
    onConfirm(destinationId, actionMode, pickerRootId)
    onClose()
  }

  const filteredFolders = search
    ? folders.filter((f) => displayName(f).toLowerCase().includes(search.toLowerCase()))
    : folders

  // Folders currently being moved cannot be a destination, nor can the user
  // navigate INTO them — doing so would let you move a folder into itself or
  // its own subtree (a cycle). Opening is the only way into a moved folder's
  // descendants in this UI (search only filters the current directory), so
  // blocking open is what prevents reaching the subtree.
  const movedFolderIds = new Set(items.filter((i) => i.isFolder).map((i) => i.id))

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      onClick={onClose}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-ink/20" />

      {/* Modal */}
      <div
        ref={focusTrapRef}
        role="dialog"
        aria-modal="true"
        aria-label={`${initialMode === 'move' ? 'Move' : 'Copy'} ${items.length} item${items.length !== 1 ? 's' : ''}`}
        onClick={(e) => e.stopPropagation()}
        className="relative w-full max-w-[520px] bg-paper border border-line-2 rounded-xl shadow-3 overflow-hidden"
      >
        {/* Header */}
        <div className="px-xl py-lg border-b border-line flex items-center gap-2.5">
          <Icon name="folder" size={14} className="text-amber-deep" />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-semibold text-ink">
              {initialMode === 'move' ? 'Move' : 'Copy'} {items.length} item{items.length !== 1 ? 's' : ''}
            </div>
            <div className="text-[11px] text-ink-3">
              Destination will re-encrypt with the folder's key
            </div>
          </div>
          <button
            onClick={onClose}
            aria-label="Close"
            className="w-7 h-7 rounded-md bg-paper-2 flex items-center justify-center text-ink-3 hover:text-ink transition-colors"
          >
            <Icon name="x" size={13} />
          </button>
        </div>

        {/* Selected items strip */}
        <div className="px-xl py-2.5 bg-paper-2 border-b border-line flex gap-1.5 flex-wrap">
          {items.map((item) => (
            <BBChip key={item.id}>
              <span className="flex items-center gap-1 text-[10.5px]">
                <Icon name={item.isFolder ? 'folder' : 'file'} size={10} />
                {item.name}
              </span>
            </BBChip>
          ))}
        </div>

        {/* Breadcrumb */}
        <div className="px-xl py-2.5 flex items-center gap-1.5 text-xs border-b border-line">
          {breadcrumbs.map((crumb, i) => {
            const isLast = i === breadcrumbs.length - 1
            return (
              <span key={i} className="flex items-center gap-1.5">
                {i > 0 && (
                  <Icon name="chevron-right" size={11} className="text-ink-4" />
                )}
                {isLast ? (
                  <span className="font-semibold text-ink">
                    {crumb.id ? pickerFolderLabel(crumb.id, crumb.name) : crumb.name}
                  </span>
                ) : (
                  <button
                    onClick={() => handleBreadcrumbNav(i)}
                    className="text-amber-deep font-medium hover:underline cursor-pointer"
                  >
                    {crumb.id ? pickerFolderLabel(crumb.id, crumb.name) : crumb.name}
                  </button>
                )}
              </span>
            )
          })}
        </div>

        {/* Search */}
        <div className="px-lg py-2.5 border-b border-line">
          <div className="flex items-center gap-2 border rounded-md bg-paper px-3 py-2 border-line focus-within:ring-2 focus-within:ring-amber/30 focus-within:border-amber-deep">
            <Icon name="search" size={12} className="text-ink-3 shrink-0" />
            <input
              ref={searchRef}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search or type a new folder name..."
              className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-4"
            />
          </div>
        </div>

        {/* Folder list */}
        <div className="max-h-60 overflow-y-auto" role="listbox" aria-label="Folders">
          {loading ? (
            <div className="px-xl py-6 text-center text-sm text-ink-3">Loading...</div>
          ) : filteredFolders.length === 0 ? (
            <div className="px-xl py-6 text-center text-sm text-ink-3">
              {search ? 'No folders found' : 'Empty folder'}
            </div>
          ) : (
            filteredFolders.map((folder) => {
              const isMoved = movedFolderIds.has(folder.id)
              const isSelected = !isMoved && selectedId === folder.id
              return (
                <button
                  key={folder.id}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  aria-disabled={isMoved}
                  disabled={isMoved}
                  className={`w-full text-left flex items-center gap-2.5 px-xl py-2.5 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-amber-deep ${
                    isMoved ? 'cursor-not-allowed text-ink-4' : 'cursor-pointer'
                  }`}
                  style={{
                    opacity: isMoved ? 0.5 : 1,
                    background: isSelected ? 'var(--color-amber-bg)' : 'transparent',
                    borderLeft: isSelected
                      ? '3px solid var(--color-amber-deep)'
                      : '3px solid transparent',
                  }}
                  onClick={() => {
                    if (isMoved) return
                    setSelectedId(folder.id)
                  }}
                  onDoubleClick={() => {
                    if (isMoved) return
                    handleFolderOpen(folder)
                  }}
                >
                  <Icon
                    name="folder"
                    size={14}
                    className={
                      isMoved ? 'text-ink-4' : isSelected ? 'text-amber-deep' : 'text-ink-3'
                    }
                  />
                  <span
                    className={`flex-1 text-[13px] ${isSelected ? 'font-semibold' : ''}`}
                  >
                    {pickerFolderLabel(folder.id, displayName(folder))}
                  </span>
                  {isMoved ? (
                    <span className="text-[10.5px] text-ink-4 whitespace-nowrap">
                      Can't move here
                    </span>
                  ) : (
                    isSelected && (
                      <Icon name="check" size={13} className="text-amber-deep" />
                    )
                  )}
                </button>
              )
            })
          )}
        </div>

        {/* Footer */}
        <div className="px-xl py-md border-t border-line flex items-center gap-2.5">
          <button
            type="button"
            onClick={handleNewFolder}
            disabled={creatingFolder}
            className="flex items-center gap-1.5 text-[11.5px] text-ink-3 hover:text-ink transition-colors cursor-pointer"
          >
            <Icon name="plus" size={11} />
            New folder
          </button>
          <div className="ml-auto flex gap-2">
            {/* "Copy here" removed (QA #1): there is no server-side copy — the
                button performed a MOVE, silently losing the original. Re-add only
                alongside a real copy/duplicate endpoint. */}
            <BBButton
              size="sm"
              variant="amber"
              onClick={() => handleConfirm('move')}
            >
              Move here
            </BBButton>
          </div>
        </div>
      </div>
    </div>
  )
}
