// The drive file list: rows, selection, and per-row actions.
import { useState, useEffect, useCallback, useRef, useMemo, type ReactNode } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { BBButton, FileRowSkeleton } from '../design'
import { Icon } from './icons'
import { FileRow, type FileRowProps } from './file-list/file-row'
import { FileListHeader } from './file-list/file-list-header'
import { ContextMenu } from './context-menu'
import {
  clearPendingSelectTimer,
  handleRowClick as resolveRowClick,
  handleRowInteractionCapture,
  handleRowDoubleClick as resolveRowDoubleClick,
} from './lib/pending-select-timer'
import { modKey } from './lib/mod-key'
import { isPreviewable } from './lib/preview'
import { getFolderColorDot, FOLDER_COLORS } from './lib/folder-colors'
import type { DriveFile, MyShare } from '../types'
import { SORT_KEY_PREFIX, SORT_OPTIONS, typeGroupRank, type SortKey } from './file-list-sort'

/** A notice the list asks its host to show (web: its toast). */
export interface FileListNotice {
  icon?: string
  title: string
  description?: string
  danger?: boolean
}

/**
 * Everything the list needs from outside: decrypted names, thumbnails, rename, pins, the id class of a row
 * and a share popover. Web implements it with its keys and API; desktop with DriveSource. The hook members
 * are called on every render, so the object must keep its identity.
 */
export interface FileListHost {
  isUnlocked: boolean
  useDecryptedNames(files: DriveFile[], external?: Record<string, string>): Record<string, string | null>
  displayNameFrom(names: Record<string, string | null>, file: DriveFile): string | null
  useThumbnails(isUnlocked: boolean): { thumbnails: Record<string, string>; setThumbRef(fileId: string, el: HTMLElement | null): void }
  /** Subscribe to "this file was just decrypted locally"; returns the unsubscribe. */
  onDecrypted(listener: (fileId: string) => void): () => void
  pinnedFolderIds: string[]
  setPinnedFolderIds(ids: string[]): Promise<void>
  /** Renames one row; throws a refusal the host words via `renameFailureMessage`. */
  renameFile(file: DriveFile, newName: string): Promise<void>
  renameFailureMessage(err: unknown): string | null
  /** Asks for the id class of the ids once; resolves when `isForeign` can answer. */
  classifyItemIds(ids: string[]): Promise<void>
  isForeign(id: string): boolean
  notify(notice: FileListNotice): void
  /** The anchored share popover opened from a row's share badge. */
  renderSharePopover?(args: { fileId: string; anchorRect: DOMRect; onClose: () => void }): ReactNode
}

// ─── Helpers ─────────────────────────────────────────

/** Relative age ("3h ago"); lives in lib/format so the guest page shares it. */

function dateGroup(dateStr: string): string {
  const date = new Date(dateStr)
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today)
  yesterday.setDate(yesterday.getDate() - 1)
  const weekAgo = new Date(today)
  weekAgo.setDate(weekAgo.getDate() - 7)
  const fileDay = new Date(date.getFullYear(), date.getMonth(), date.getDate())
  if (fileDay >= today) return 'Today'
  if (fileDay >= yesterday) return 'Yesterday'
  if (fileDay >= weekAgo) return 'This week'
  return 'Earlier'
}

// ─── Props ───────────────────────────────────────────


export interface FileListProps {
  host: FileListHost
  files: DriveFile[]
  loading: boolean
  emptyState: ReactNode
  onRefresh?: () => void
  sortable?: boolean
  selectable?: boolean
  showDateGroups?: boolean
  /** Inline upload cards rendered at the top of the scrollable area (spec 024 §4). */
  uploadCards?: ReactNode
  /**
   * Current folder ID (or null / undefined for root). Used to key sort
   * persistence in localStorage so each folder remembers its own sort order.
   */
  parentId?: string | null

  // Navigation
  onNavigateFolder?: (folder: DriveFile) => void

  // File selection (drives detail panel)
  selectedFileId?: string | null
  onSelectFile?: (file: DriveFile | null) => void

  // Selection change for external keyboard shortcuts
  onSelectionChange?: (ids: Set<string>) => void

  // Decrypted name sync for dialog use in parent
  onDecryptedNamesChange?: (names: Record<string, string | null>) => void

  // Context menu / row action handler
  onFileAction?: (action: string, file: DriveFile) => void

  // Direct star toggle (bypasses context menu, used by inline star button)
  onToggleStar?: (fileId: string) => void

  // Drag-and-drop to folder handler
  onDropToFolder?: (draggedIds: string[], targetFolder: DriveFile) => Promise<void>

  // Custom row actions (replaces the kebab button)
  renderActions?: (file: DriveFile) => ReactNode

  // Bulk action callbacks (showing bulk bar requires at least one non-undefined)
  onBulkTrash?: (ids: string[]) => Promise<void>
  onBulkMove?: (ids: string[]) => void
  onBulkShare?: (ids: string[]) => void
  onBulkDownload?: (ids: string[]) => Promise<void>
  /** When set, shows ZIP progress in the bulk action bar and disables the download button. */
  bulkZipProgress?: { done: number; total: number } | null

  // Animation state driven by parent (for drive page micro-interactions)
  trashingIds?: Set<string>
  starPulseId?: string | null
  recentlyUploadedIds?: Set<string>

  // Pre-decrypted names override (for Shared page with custom key derivation)
  externalDecryptedNames?: Record<string, string>

  // Trust details: clicking the lock icon opens this callback's panel.
  onShowTrustDetails?: (file: DriveFile) => void

  /** Active shares owned by the current user, used to flag rows that have at
   *  least one non-revoked share when `share_count` may not be populated yet
   *  (e.g. immediately after a share is created). */
  myShares?: MyShare[]

  /**
   * Viewer count per folder ID. When a folder has > 0 members, a compact
   * presence badge is shown in its name cell.
   * Shape: { [folderId]: { count: number; emails: string[] } }
   */
  folderViewerCounts?: Record<string, { count: number; emails: string[] }>
}

// ─── Component ───────────────────────────────────────

export function FileList({
  host,
  files,
  loading,
  emptyState,
  sortable = true,
  selectable = true,
  showDateGroups = false,
  onNavigateFolder,
  selectedFileId,
  onSelectFile,
  onSelectionChange,
  onDecryptedNamesChange,
  onFileAction,
  onDropToFolder,
  onToggleStar,
  renderActions,
  onBulkTrash,
  onBulkMove,
  onBulkShare,
  onBulkDownload,
  bulkZipProgress = null,
  trashingIds = new Set<string>(),
  starPulseId = null,
  recentlyUploadedIds = new Set<string>(),
  externalDecryptedNames,
  onShowTrustDetails,
  uploadCards,
  myShares,
  folderViewerCounts,
  parentId,
  onRefresh,
}: FileListProps) {
  const { isUnlocked } = host
  const showToast = host.notify

  // ─── Active-share lookup ───────────────────────────
  // Derived set of file ids that currently have at least one non-revoked,
  // non-expired share belonging to the current user. Used to show the share
  // indicator next to filenames even when `file.share_count` is stale (the
  // server count can lag right after a share is created/revoked).
  const sharedFileIds = useMemo(() => {
    if (!myShares) return null
    const now = Date.now()
    const ids = new Set<string>()
    for (const s of myShares) {
      if (s.revoked) continue
      if (s.expires_at) {
        const exp = new Date(s.expires_at).getTime()
        if (Number.isFinite(exp) && exp <= now) continue
      }
      ids.add(s.file_id)
    }
    return ids
  }, [myShares])

  // ─── Sort state ────────────────────────────────────
  // Key per folder: beebeeb.sort.<folderId> or beebeeb.sort.root
  // Falls back to the old global key for a smooth migration.
  const sortLsKey = `${SORT_KEY_PREFIX}${parentId ?? 'root'}`

  const [sortKey, setSortKey] = useState<SortKey>(() => {
    try {
      // Try per-folder key first, then fall back to old global key
      const raw = localStorage.getItem(sortLsKey) ?? localStorage.getItem('bb_drive_sort')
      if (raw) {
        const parsed = JSON.parse(raw) as { key?: string }
        if (parsed.key && SORT_OPTIONS.some((o) => o.key === parsed.key)) {
          return parsed.key as SortKey
        }
      }
    } catch { /* fall through */ }
    return 'name-asc'
  })
  const [sortMenuOpen, setSortMenuOpen] = useState(false)

  // Re-read from localStorage when the folder changes (parentId changed)
  const prevSortLsKeyRef = useRef(sortLsKey)
  useEffect(() => {
    if (prevSortLsKeyRef.current === sortLsKey) return
    prevSortLsKeyRef.current = sortLsKey
    try {
      const raw = localStorage.getItem(sortLsKey) ?? localStorage.getItem('bb_drive_sort')
      if (raw) {
        const parsed = JSON.parse(raw) as { key?: string }
        if (parsed.key && SORT_OPTIONS.some((o) => o.key === parsed.key)) {
          setSortKey(parsed.key as SortKey)
          return
        }
      }
    } catch { /* ignore */ }
    setSortKey('name-asc')
  }, [sortLsKey])

  useEffect(() => {
    try {
      localStorage.setItem(sortLsKey, JSON.stringify({ key: sortKey }))
    } catch { /* ignore */ }
  }, [sortKey, sortLsKey])

  // ─── Decrypted names ───────────────────────────────
  const decryptedNames = host.useDecryptedNames(files, externalDecryptedNames)

  useEffect(() => {
    onDecryptedNamesChange?.(decryptedNames)
  }, [decryptedNames, onDecryptedNamesChange])

  const displayName = (file: DriveFile) => host.displayNameFrom(decryptedNames, file)

  // ─── Thumbnails ────────────────────────────────────
  const { thumbnails, setThumbRef } = host.useThumbnails(isUnlocked)

  // ─── Selection ─────────────────────────────────────
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set<string>())
  const lastClickedIdRef = useRef<string | null>(null)
  // Task 1565 preview matrix: root-caused live (a real .doc upload's dblclick
  // never opened anything — not the preview, not even the "can't be
  // previewed" toast). A double-click delivers two native 'click' events
  // before the browser's own 'dblclick' — for a NON-previewable file, click
  // #1's onClick below calls onSelectFile synchronously, which mounts
  // FileDetailsPanel's full-viewport `fixed inset-0 z-40` click-to-close
  // backdrop. If that mount lands (React commit + paint) in the gap before
  // click #2 arrives — confirmed live via a capture-phase event-log repro:
  // click #1 hit the row's own metadata text node, click #2 hit
  // `absolute inset-0 bg-ink/10` (the backdrop) instead — the SECOND click
  // never reaches the row at all, so the browser never fires 'dblclick' on
  // it (both clicks must target the same element), and neither
  // onDoubleClick's toast nor a preview ever appears. This is timing-
  // dependent (worse under CPU load, which is exactly when it was caught),
  // so a real user double-clicking a legacy .doc/.ppt/.xls/.zip/binary file
  // could silently see nothing happen. Fix: debounce the SELECT side effect
  // (which is what mounts the backdrop) by the standard OS double-click
  // window, and cancel it if a dblclick follows — the previewable-file path
  // below is untouched because a single click there already opens the
  // preview immediately, so no backdrop-timing race exists for it.
  //
  // Follow-up bug (found live after #107 shipped the debounce above): the
  // original fix only cleared this timer from the SAME row's own
  // onDoubleClick. A single click on a folder or a previewable file, a
  // shift/mod-key click, or keyboard activation of a DIFFERENT row within
  // the 300ms window left the stale timer armed — it fired afterward and
  // opened the FIRST file's details panel over whatever the user had since
  // navigated to or opened. Fixed by moving all the click/dblclick decision
  // logic (and the ONE unconditional clear every entry point now performs
  // first) into src/lib/pending-select-timer.ts — see that module's header
  // comment for the full case list and the onKeyDown clear below.
  const pendingSelectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    return () => clearPendingSelectTimer(pendingSelectTimerRef)
  }, [])

  const [inlineRenameId, setInlineRenameId] = useState<string | null>(null)
  const [inlineRenameValue, setInlineRenameValue] = useState('')
  const inlineRenameInputRef = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    if (inlineRenameId && inlineRenameInputRef.current) {
      inlineRenameInputRef.current.focus()
      const dotIdx = inlineRenameValue.lastIndexOf('.')
      inlineRenameInputRef.current.setSelectionRange(0, dotIdx > 0 ? dotIdx : inlineRenameValue.length)
    }
  }, [inlineRenameId]) // eslint-disable-line react-hooks/exhaustive-deps

  const updateSelection = useCallback((next: Set<string>) => {
    setSelectedIds(next)
    onSelectionChange?.(next)
  }, [onSelectionChange])

  function toggleSelection(fileId: string) {
    const next = new Set(selectedIds)
    if (next.has(fileId)) next.delete(fileId)
    else next.add(fileId)
    updateSelection(next)
    lastClickedIdRef.current = fileId
  }

  const clearSelection = useCallback(() => {
    updateSelection(new Set<string>())
    lastClickedIdRef.current = null
  }, [updateSelection])

  // ─── Decryption pulse ──────────────────────────────
  // When a file in the list is decrypted (preview/download), briefly pulse
  // the lock chip to show "something secure happened locally".
  const [decryptPulseId, setDecryptPulseId] = useState<string | null>(null)
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    const off = host.onDecrypted((fileId) => {
      setDecryptPulseId(fileId)
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => setDecryptPulseId(null), 900)
    })
    return () => {
      off()
      if (timer) clearTimeout(timer)
    }
  }, [host])

  // ─── Pinned folders (from shared DriveDataContext) ────────────────────────
  const contextPinnedIds = host.pinnedFolderIds
  const pinnedFolderIds = new Set(contextPinnedIds)

  const MAX_PINS = 10

  async function handleTogglePin(folderId: string) {
    const current = contextPinnedIds
    const isPinning = !current.includes(folderId)
    if (isPinning && current.length >= MAX_PINS) {
      showToast({
        icon: 'bell',
        title: 'Pin limit reached',
        description: 'Remove a pin first to add a new one.',
      })
      return
    }
    const newIds = isPinning
      ? [...current, folderId]
      : current.filter(id => id !== folderId)
    await host.setPinnedFolderIds(newIds).catch(() => {})
  }

  // ─── Folder color labels ───────────────────────────
  // Map of folderId → hex dot color. Initialized lazily from localStorage when
  // files load. Updated immediately on user color selection without page reload.
  const [folderColors, setFolderColors] = useState<Record<string, string | null>>({})

  useEffect(() => {
    if (files.length === 0) return
    const initial: Record<string, string | null> = {}
    for (const file of files) {
      if (file.is_folder) {
        initial[file.id] = getFolderColorDot(file.id)
      }
    }
    setFolderColors(initial)
  }, [files])

  function handleFolderColorChange(folderId: string, colorName: string | null) {
    const dot = colorName
      ? (FOLDER_COLORS.find((c) => c.name === colorName)?.dot ?? null)
      : null
    setFolderColors((prev) => ({ ...prev, [folderId]: dot }))
    // Notify quick-access and other listeners so they can re-read localStorage
    window.dispatchEvent(new CustomEvent('beebeeb:folder-color-changed', { detail: { folderId } }))
  }

  // ─── Context menu ──────────────────────────────────
  const [ctxMenu, setCtxMenu] = useState<{
    open: boolean; x: number; y: number; fileId: string; fileName: string; isFolder: boolean; versionNumber: number; isStarred: boolean; mimeType: string | null
  }>({ open: false, x: 0, y: 0, fileId: '', fileName: '', isFolder: false, versionNumber: 1, isStarred: false, mimeType: null })

  // ─── Share popover ────────────────────────────────
  const [sharePopover, setSharePopover] = useState<{
    fileId: string
    anchorRect: DOMRect
  } | null>(null)

  // ─── Drag-and-drop ─────────────────────────────────
  const [draggedFileId, setDraggedFileId] = useState<string | null>(null)
  const [dragOverFolderId, setDragOverFolderId] = useState<string | null>(null)

  function handleDragStart(e: React.DragEvent, file: DriveFile) {
    const ids =
      selectedIds.size > 1 && selectedIds.has(file.id)
        ? Array.from(selectedIds)
        : [file.id]
    e.dataTransfer.setData('text/plain', JSON.stringify(ids))
    if (file.is_folder && ids.length === 1) {
      e.dataTransfer.setData('application/beebeeb-folder', file.id)
    }
    e.dataTransfer.effectAllowed = 'move'
    setDraggedFileId(file.id)

    // Custom drag image: for multi-select, show a stacked card with a count
    // badge instead of just the dragged row (which doesn't communicate that
    // multiple items are moving). For single-item drags, the default ghost
    // is fine — keep it.
    if (ids.length > 1) {
      const ghost = document.createElement('div')
      ghost.style.position = 'absolute'
      ghost.style.top = '-1000px'
      ghost.style.left = '-1000px'
      ghost.style.padding = '6px 12px 6px 14px'
      ghost.style.background = 'oklch(0.99 0.01 84)'
      ghost.style.color = 'oklch(0.20 0.02 60)'
      ghost.style.border = '1px solid oklch(0.86 0.07 90)'
      ghost.style.borderRadius = '8px'
      ghost.style.boxShadow = '0 6px 16px rgba(0,0,0,0.18), 4px 4px 0 -1px oklch(0.99 0.01 84), 4px 4px 0 0 oklch(0.86 0.07 90)'
      ghost.style.fontFamily = 'Inter, system-ui, sans-serif'
      ghost.style.fontSize = '12px'
      ghost.style.fontWeight = '500'
      ghost.style.whiteSpace = 'nowrap'
      ghost.textContent = `${ids.length} items`
      document.body.appendChild(ghost)
      e.dataTransfer.setDragImage(ghost, 12, 12)
      // Browsers snapshot the image synchronously, so we can clean up next tick.
      setTimeout(() => { ghost.remove() }, 0)
    }
  }

  function handleDragEnd() {
    setDraggedFileId(null)
    setDragOverFolderId(null)
  }

  function handleDragOver(e: React.DragEvent, folder: DriveFile) {
    if (!folder.is_folder) return
    if (draggedFileId === folder.id) return
    if (
      selectedIds.size > 1 &&
      selectedIds.has(draggedFileId ?? '') &&
      selectedIds.has(folder.id)
    ) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'move'
    setDragOverFolderId(folder.id)
  }

  function handleDragLeave(e: React.DragEvent, folderId: string) {
    const relatedTarget = e.relatedTarget as HTMLElement | null
    const currentTarget = e.currentTarget as HTMLElement
    if (relatedTarget && currentTarget.contains(relatedTarget)) return
    if (dragOverFolderId === folderId) setDragOverFolderId(null)
  }

  async function handleDrop(e: React.DragEvent, targetFolder: DriveFile) {
    e.preventDefault()
    setDragOverFolderId(null)
    setDraggedFileId(null)
    if (!targetFolder.is_folder || !onDropToFolder) return
    let fileIds: string[]
    try {
      fileIds = JSON.parse(e.dataTransfer.getData('text/plain')) as string[]
    } catch { return }
    fileIds = fileIds.filter((id) => id !== targetFolder.id)
    if (fileIds.length === 0) return
    await onDropToFolder(fileIds, targetFolder)
    clearSelection()
  }

  // ─── Sort ──────────────────────────────────────────
  // safeName(): like displayName() but returns '' while decryption is pending,
  // so sort comparators never receive null.
  function safeName(file: DriveFile): string {
    return displayName(file) ?? ''
  }

  const sortedFiles = useMemo(() => {
    if (!sortable) return files
    return [...files].sort((a, b) => {
      // starred-first: starred items come before non-starred across the board
      if (sortKey === 'starred-first') {
        const starA = a.is_starred ? 0 : 1
        const starB = b.is_starred ? 0 : 1
        if (starA !== starB) return starA - starB
        // Within same star group: folders first, then name
        if (a.is_folder && !b.is_folder) return -1
        if (!a.is_folder && b.is_folder) return 1
        return safeName(a).localeCompare(safeName(b))
      }
      // type-asc: grouped by meaningful category (image, video, doc, …), no folder pinning override
      if (sortKey === 'type-asc') {
        const ra = typeGroupRank(a, safeName(a))
        const rb = typeGroupRank(b, safeName(b))
        if (ra !== rb) return ra - rb
        return safeName(a).localeCompare(safeName(b))
      }
      // All other sorts: folders always come first
      if (a.is_folder && !b.is_folder) return -1
      if (!a.is_folder && b.is_folder) return 1
      switch (sortKey) {
        case 'name-asc':      return safeName(a).localeCompare(safeName(b))
        case 'name-desc':     return safeName(b).localeCompare(safeName(a))
        case 'modified-desc': return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()
        case 'modified-asc':  return new Date(a.updated_at).getTime() - new Date(b.updated_at).getTime()
        case 'size-desc':     return b.size_bytes - a.size_bytes
        case 'size-asc':      return a.size_bytes - b.size_bytes
      }
    })
  }, [files, sortable, sortKey, decryptedNames])

  const allSelected = sortedFiles.length > 0 && selectedIds.size === sortedFiles.length
  const someSelected = selectedIds.size > 0 && selectedIds.size < sortedFiles.length

  // T5.5: ask core for the id class of every listed row once per id. The F
  // marker then reads this client's answer (`isForeignItemById`), never the
  // server-supplied attribution columns.
  const foreignClassKey = sortedFiles.map((f) => f.id).join(',')
  const [foreignClassTick, setForeignClassTick] = useState(0)
  useEffect(() => {
    if (!foreignClassKey) return
    let cancelled = false
    void host.classifyItemIds(foreignClassKey.split(',')).then(() => {
      if (!cancelled) setForeignClassTick((n) => n + 1)
    })
    return () => { cancelled = true }
  }, [foreignClassKey, host])

  // ─── Row click handler ─────────────────────────────
  // Branching + the pending-select-timer clear/schedule decision live in
  // src/lib/pending-select-timer.ts (unit-tested there, framework-free) —
  // this wrapper only resolves the shift-range candidacy (which needs
  // sortedFiles/selectedIds) and wires each outcome to its real side effect.
  function handleRowClick(file: DriveFile, e: React.MouseEvent) {
    let isShiftRange = false
    let rangeIds: string[] = []
    let rangeStart = 0
    let rangeEnd = 0
    if (e.shiftKey && lastClickedIdRef.current) {
      const ids = sortedFiles.map((f) => f.id)
      const lastIdx = ids.indexOf(lastClickedIdRef.current)
      const curIdx = ids.indexOf(file.id)
      if (lastIdx !== -1 && curIdx !== -1) {
        isShiftRange = true
        rangeIds = ids
        rangeStart = Math.min(lastIdx, curIdx)
        rangeEnd = Math.max(lastIdx, curIdx)
      }
    }

    resolveRowClick(
      pendingSelectTimerRef,
      {
        isShiftRange,
        isModKey: !!e[modKey],
        isFolder: file.is_folder,
        isPreviewable: isPreviewable(file.mime_type, decryptedNames[file.id]),
      },
      {
        rangeSelect: () => {
          const next = new Set(selectedIds)
          for (const id of rangeIds.slice(rangeStart, rangeEnd + 1)) next.add(id)
          updateSelection(next)
        },
        toggleSelect: () => toggleSelection(file.id),
        navigateFolder: () => onNavigateFolder?.(file),
        preview: () => onFileAction?.('preview', file),
        // Debounced (see pendingSelectTimerRef's doc comment above): a bare
        // synchronous onSelectFile here mounts FileDetailsPanel's full-viewport
        // click-to-close backdrop before this click's own dblclick companion
        // (if any) can land on the row, silently eating the double-click.
        selectFile: () => onSelectFile?.(file),
      },
    )
  }

  function handleCheckboxClick(fileId: string, e: React.MouseEvent) {
    e.stopPropagation()
    if (e.shiftKey && lastClickedIdRef.current) {
      const ids = sortedFiles.map((f) => f.id)
      const lastIdx = ids.indexOf(lastClickedIdRef.current)
      const curIdx = ids.indexOf(fileId)
      if (lastIdx !== -1 && curIdx !== -1) {
        const start = Math.min(lastIdx, curIdx)
        const end = Math.max(lastIdx, curIdx)
        const next = new Set(selectedIds)
        for (const id of ids.slice(start, end + 1)) next.add(id)
        updateSelection(next)
        lastClickedIdRef.current = fileId
        return
      }
    }
    toggleSelection(fileId)
  }

  // ─── Keyboard shortcuts: select all + escape ───────
  useEffect(() => {
    if (!selectable) return
    function onKey(e: KeyboardEvent) {
      if (e.target instanceof HTMLInputElement) return
      if (e.target instanceof HTMLTextAreaElement) return
      if (e[modKey] && e.key === 'a') {
        e.preventDefault()
        updateSelection(new Set(sortedFiles.map((f) => f.id)))
      } else if (e.key === 'Escape' && selectedIds.size > 0) {
        clearSelection()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [selectable, sortedFiles, selectedIds, updateSelection, clearSelection])

  // ─── Date grouping ─────────────────────────────────
  const grouped: { label: string; files: DriveFile[] }[] | null = showDateGroups
    ? (() => {
        const order = ['Today', 'Yesterday', 'This week', 'Earlier']
        const groups: Record<string, DriveFile[]> = {}
        for (const file of sortedFiles) {
          const label = dateGroup(file.updated_at)
          if (!groups[label]) groups[label] = []
          groups[label].push(file)
        }
        return order
          .filter((label) => groups[label]?.length ?? 0 > 0)
          .map((label) => ({ label, files: groups[label] }))
      })()
    : null

  // ─── Virtualization ────────────────────────────────
  // Window the file rows so 10k+ files render at 60fps. Only visible rows + a
  // small overscan are mounted in the DOM at any time. We virtualize a flat
  // list of items — either a date-group header (in showDateGroups mode) or a
  // file row — keyed by index. The scroll container is the body div below.
  type VirtualItem =
    | { kind: 'header'; label: string }
    | { kind: 'row'; file: DriveFile; rowIndex: number }

  const virtualItems = useMemo<VirtualItem[]>(() => {
    if (grouped) {
      const out: VirtualItem[] = []
      let rowIdx = 0
      for (const { label, files: groupFiles } of grouped) {
        out.push({ kind: 'header', label })
        for (const file of groupFiles) {
          out.push({ kind: 'row', file, rowIndex: rowIdx++ })
        }
      }
      return out
    }
    return sortedFiles.map((file, i) => ({ kind: 'row', file, rowIndex: i }))
  }, [grouped, sortedFiles])

  // Map from sortedFiles index → virtualItems index. Used by keyboard arrow
  // navigation to translate a file's position into the right virtual item to
  // scroll/focus.
  const fileIndexToVirtualIndex = useMemo(() => {
    const map = new Map<number, number>()
    for (let i = 0; i < virtualItems.length; i++) {
      const item = virtualItems[i]
      if (item.kind === 'row') map.set(item.rowIndex, i)
    }
    return map
  }, [virtualItems])

  const scrollContainerRef = useRef<HTMLDivElement | null>(null)

  // Rough estimate from the row's styling: py-[11px] (22px vertical padding) +
  // ~50px content (name, type/badge subline, AES line). measureElement
  // refines this per-row once mounted, so the estimate only affects initial
  // scroll positioning.
  const ROW_HEIGHT_ESTIMATE = 72
  const HEADER_HEIGHT_ESTIMATE = 32

  const rowVirtualizer = useVirtualizer({
    count: virtualItems.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: (i) => virtualItems[i]?.kind === 'header' ? HEADER_HEIGHT_ESTIMATE : ROW_HEIGHT_ESTIMATE,
    overscan: 8,
    // Stable keys so re-orderings (sort changes) don't remount unrelated rows.
    getItemKey: (i) => {
      const item = virtualItems[i]
      return item?.kind === 'header' ? `header:${item.label}` : `row:${item.file.id}`
    },
  })

  // Focus the row at the given sortedFiles index. With virtualization the
  // target row may not be in the DOM yet — scroll the virtualizer to it first,
  // then focus on the next paint once the row mounts. Used by ArrowUp/Down.
  const focusRowAtIndex = useCallback((targetRowIndex: number) => {
    if (targetRowIndex < 0 || targetRowIndex >= sortedFiles.length) return
    const virtualIdx = fileIndexToVirtualIndex.get(targetRowIndex)
    if (virtualIdx == null) return
    rowVirtualizer.scrollToIndex(virtualIdx, { align: 'auto' })
    // Defer focus to next frame so the row has time to mount after scrolling.
    requestAnimationFrame(() => {
      const el = scrollContainerRef.current?.querySelector<HTMLElement>(
        `[data-row-index="${targetRowIndex}"]`,
      )
      if (el) el.focus()
    })
  }, [sortedFiles.length, fileIndexToVirtualIndex, rowVirtualizer])

  async function commitInlineRename(file: DriveFile) {
    const newName = inlineRenameValue.trim()
    setInlineRenameId(null)
    const currentName = displayName(file)
    if (!newName || newName === currentName) return
    if (!isUnlocked) {
      showToast({ icon: 'lock', title: 'Vault is locked', description: 'Unlock to rename files.', danger: true })
      return
    }
    try {
      await host.renameFile(file, newName)
      onRefresh?.()
    } catch (err) {
      showToast({
        icon: 'x',
        title: 'Rename failed',
        description: host.renameFailureMessage(err) ?? 'Could not rename the file.',
        danger: true,
      })
    }
  }

  // ─── Row props ─────────────────────────────────────
  // `rowIndex` is the file's index into `sortedFiles`. With virtualization, the
  // DOM no longer contains adjacent rows for nextElementSibling-style traversal,
  // so we use data-row-index to look up rows by position instead. The row
  // itself is `FileRow`; all state and side effects stay here.
  function rowFor(file: DriveFile, rowIndex: number): FileRowProps {
    // null = decryption still running for this file — show skeleton in name cell
    const name = displayName(file)
    return {
      file,
      rowIndex,
      name,
      selection: {
        selectable,
        isSelected: selectedIds.has(file.id),
        isCurrent: selectedFileId === file.id,
        anySelected: selectedIds.size > 0,
      },
      flags: {
        isDragTarget: file.is_folder && dragOverFolderId === file.id,
        isDragging: draggedFileId === file.id,
        isTrashing: trashingIds.has(file.id),
        isRecentUpload: recentlyUploadedIds.has(file.id),
        isPinned: file.is_folder && pinnedFolderIds.has(file.id),
        isStarPulse: starPulseId === file.id,
        isDecryptPulse: decryptPulseId === file.id,
        isUnlocked,
        hasActiveShare: (file.share_count ?? 0) > 0 || !!sharedFileIds?.has(file.id),
        canShowTrustDetails: !!onShowTrustDetails && !file.is_folder,
        isForeign: foreignClassTick >= 0 && host.isForeign(file.id),
      },
      thumbnail: {
        enabled: !!(file.has_thumbnail && isUnlocked),
        url: thumbnails[file.id],
        folderColor: folderColors[file.id] ?? null,
        viewerCount: folderViewerCounts?.[file.id],
      },
      inlineRename: {
        editing: inlineRenameId === file.id,
        value: inlineRenameValue,
        inputRef: inlineRenameInputRef,
        onChange: setInlineRenameValue,
        commit: () => { void commitInlineRename(file) },
        cancel: () => setInlineRenameId(null),
      },
      renderActions,
      handlers: {
        onDragStart: (e) => handleDragStart(e, file),
        onDragEnd: handleDragEnd,
        onDragOver: (e) => handleDragOver(e, file),
        onDragLeave: (e) => handleDragLeave(e, file.id),
        onDrop: (e) => { void handleDrop(e, file) },
        // Capture phase: runs before ANY nested control's onClick (checkbox,
        // star, share badges, lock, row actions, rename input — all of which
        // stopPropagation and so never reach the row's onClick), clearing a
        // debounced select-timer armed by an earlier non-previewable click.
        // See handleRowInteractionCapture in pending-select-timer.ts.
        onInteractionCapture: () => handleRowInteractionCapture(pendingSelectTimerRef),
        onClick: (e) => handleRowClick(file, e),
        onDoubleClick: (e) => {
          e.stopPropagation()
          // A real dblclick DID land on this row (both native clicks reached
          // it) — cancel the debounced onSelectFile from the first click (of
          // THIS row or any other) so the details panel doesn't ALSO pop
          // open right after this fires. See pending-select-timer.ts.
          resolveRowDoubleClick(
            pendingSelectTimerRef,
            { isFolder: file.is_folder, isPreviewable: isPreviewable(file.mime_type, decryptedNames[file.id]) },
            {
              navigateFolder: () => onNavigateFolder?.(file),
              preview: () => onFileAction?.('preview', file),
              cannotPreviewToast: () => showToast({
                icon: 'file',
                title: "This file type can't be previewed",
                description: 'Use Open to download it.',
              }),
            },
          )
        },
        onContextMenu: (e) => {
          e.preventDefault()
          // A right-click is a new action too — don't let a stale debounced
          // select pop a details panel over the context menu it opens.
          handleRowInteractionCapture(pendingSelectTimerRef)
          setCtxMenu({
            open: true,
            x: e.clientX,
            y: e.clientY,
            fileId: file.id,
            fileName: name ?? 'Encrypted file',
            isFolder: file.is_folder,
            versionNumber: file.version_number ?? 1,
            isStarred: file.is_starred ?? false,
            mimeType: file.mime_type ?? null,
          })
        },
        onKeyDown: (e) => {
          // Keyboard activation of ANY row must not let a stale debounced
          // select-timer from an earlier mouse click (on this row or a
          // different one) fire afterward — see pending-select-timer.ts.
          clearPendingSelectTimer(pendingSelectTimerRef)
          // Enter: open folder or preview file
          if (e.key === 'Enter') {
            e.preventDefault()
            if (file.is_folder) onNavigateFolder?.(file)
            else onFileAction?.('open', file)
          }
          // Space: toggle selection
          else if (e.key === ' ') {
            e.preventDefault()
            toggleSelection(file.id)
          }
          // Context menu key or Shift+F10: open context menu at element position
          else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
            e.preventDefault()
            const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
            setCtxMenu({
              open: true,
              x: rect.left + 8,
              y: rect.bottom + 2,
              fileId: file.id,
              fileName: name ?? 'Encrypted file',
              isFolder: file.is_folder,
              versionNumber: file.version_number ?? 1,
              isStarred: file.is_starred ?? false,
              mimeType: file.mime_type ?? null,
            })
          }
          // Delete / Backspace: trash the focused file (single-file keyboard delete).
          // stopPropagation prevents the global useKeyboardShortcuts handler from
          // also firing onTrashSelected for the bulk-selection set.
          else if (e.key === 'Delete' || e.key === 'Backspace') {
            e.preventDefault()
            e.stopPropagation()
            onFileAction?.('trash', file)
          }
          // Arrow keys: move focus to adjacent rows + scroll into view.
          // With virtualization, adjacent rows may not be in the DOM (only
          // visible + overscan are mounted), so we navigate by row index and
          // ask the virtualizer to scroll the target into view, then focus it
          // after it mounts.
          else if (e.key === 'ArrowDown') {
            e.preventDefault()
            focusRowAtIndex(rowIndex + 1)
          }
          else if (e.key === 'ArrowUp') {
            e.preventDefault()
            focusRowAtIndex(rowIndex - 1)
          }
        },
        onCheckboxClick: (e) => { if (selectable) handleCheckboxClick(file.id, e) },
        onToggleStar: (e) => { e.stopPropagation(); onToggleStar?.(file.id) },
        onShowTrustDetails: (e) => { e.stopPropagation(); onShowTrustDetails?.(file) },
        onOpenShare: (e) => {
          e.stopPropagation()
          const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
          setSharePopover({ fileId: file.id, anchorRect: rect })
        },
        onOpenMenu: (e) => {
          e.stopPropagation()
          const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
          setCtxMenu({
            open: true,
            x: rect.left,
            y: rect.bottom + 4,
            fileId: file.id,
            fileName: name ?? 'Encrypted file',
            isFolder: file.is_folder,
            versionNumber: file.version_number ?? 1,
            isStarred: file.is_starred ?? false,
            mimeType: file.mime_type ?? null,
          })
        },
        setThumbRef: (el) => setThumbRef(file.id, el),
      },
    }
  }


  const hasBulkBar = selectable && (onBulkTrash ?? onBulkMove ?? onBulkShare ?? onBulkDownload)

  return (
    <>
      <FileListHeader
        sortKey={sortKey}
        sortMenuOpen={sortMenuOpen}
        onToggleSortMenu={() => setSortMenuOpen((v) => !v)}
        onCloseSortMenu={() => setSortMenuOpen(false)}
        onSelectSort={(key) => { setSortKey(key); setSortMenuOpen(false) }}
        onToggleNameSort={() => setSortKey((k) => k === 'name-asc' ? 'name-desc' : 'name-asc')}
        onToggleSizeSort={() => setSortKey((k) => k === 'size-desc' ? 'size-asc' : 'size-desc')}
        onToggleModifiedSort={() => setSortKey((k) => k === 'modified-desc' ? 'modified-asc' : 'modified-desc')}
        selectable={selectable}
        totalCount={sortedFiles.length}
        allSelected={allSelected}
        someSelected={someSelected}
        onToggleAll={() => {
          if (allSelected) clearSelection()
          else updateSelection(new Set(sortedFiles.map((f) => f.id)))
        }}
      />

      {/* File list body — role=rowgroup + aria-label lets screen readers announce the list.
          This div is the scroll container; the virtualizer measures it. Upload
          cards and loading/empty fallbacks render normally above the virtualized
          rows — only the file rows themselves are windowed. */}
      <div
        ref={scrollContainerRef}
        role="rowgroup"
        aria-label="Files and folders"
        className="flex-1 overflow-y-auto min-h-0"
        onClick={(e) => {
          if (e.target === e.currentTarget && selectedIds.size > 0) clearSelection()
        }}
      >
        {/* Inline upload cards — rendered at top of scroll area (spec 024 §4) */}
        {uploadCards}
        {loading ? (
          <div aria-busy="true" aria-label="Loading files">{Array.from({ length: 8 }, (_, i) => <FileRowSkeleton key={i} />)}</div>
        ) : sortedFiles.length === 0 ? (
          emptyState
        ) : (
          // Spacer div sized to the total virtual content height; absolutely
          // positioned children are placed at their respective y-offsets.
          // Clicking the spacer (between rows or below the last row) clears
          // the selection, matching the pre-virtualization behaviour.
          <div
            onClick={(e) => {
              if (e.target === e.currentTarget && selectedIds.size > 0) clearSelection()
            }}
            style={{
              height: `${rowVirtualizer.getTotalSize()}px`,
              width: '100%',
              position: 'relative',
            }}
          >
            {rowVirtualizer.getVirtualItems().map((vi) => {
              const item = virtualItems[vi.index]
              if (!item) return null
              return (
                <div
                  key={vi.key}
                  data-index={vi.index}
                  ref={rowVirtualizer.measureElement}
                  style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    width: '100%',
                    transform: `translateY(${vi.start}px)`,
                  }}
                >
                  {item.kind === 'header' ? (
                    <div className="px-3 md:px-5 py-2 text-[11px] font-semibold text-ink-3 uppercase tracking-wider bg-paper-2 border-b border-line">
                      {item.label}
                    </div>
                  ) : (
                    <FileRow {...rowFor(item.file, item.rowIndex)} />
                  )}
                </div>
              )
            })}
          </div>
        )}
      </div>

      {/* Bulk action bar */}
      {hasBulkBar && selectedIds.size > 0 && (
        <div className="relative z-[10000] px-3 md:px-5 py-2.5 border-t border-line bg-ink flex items-center gap-2 md:gap-3.5 animate-slide-in-up">
          <span className="text-sm font-medium text-paper">{selectedIds.size} selected</span>
          <button
            onClick={clearSelection}
            className="text-xs text-paper/60 hover:text-paper transition-colors cursor-pointer"
          >
            Clear
          </button>
          <div className="ml-auto flex items-center gap-1.5">
            {onBulkMove && (
              <BBButton
                size="sm"
                variant="ghost"
                className="text-paper/80 hover:text-paper hover:bg-white/10 gap-1.5"
                onClick={() => onBulkMove(Array.from(selectedIds))}
              >
                <Icon name="folder" size={13} /> Move
              </BBButton>
            )}
            {onBulkTrash && (
              <BBButton
                size="sm"
                variant="ghost"
                className="text-paper/80 hover:text-paper hover:bg-white/10 gap-1.5"
                onClick={async () => {
                  const ids = Array.from(selectedIds)
                  await onBulkTrash(ids)
                  clearSelection()
                }}
              >
                <Icon name="trash" size={13} /> Trash
              </BBButton>
            )}
            {onBulkShare && (
              <BBButton
                size="sm"
                variant="ghost"
                className="text-paper/80 hover:text-paper hover:bg-white/10 gap-1.5"
                onClick={() => onBulkShare(Array.from(selectedIds))}
              >
                <Icon name="share" size={13} /> Share
              </BBButton>
            )}
            {onBulkDownload && (
              <BBButton
                size="sm"
                variant="amber"
                className="gap-1.5 min-w-[100px]"
                disabled={!!bulkZipProgress}
                onClick={async () => {
                  if (bulkZipProgress) return
                  const ids = Array.from(selectedIds)
                  await onBulkDownload(ids)
                }}
              >
                {bulkZipProgress ? (
                  <>
                    <svg className="animate-spin h-3 w-3 shrink-0" viewBox="0 0 24 24" fill="none">
                      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
                      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                    </svg>
                    {bulkZipProgress.total > 0
                      ? `${bulkZipProgress.done}/${bulkZipProgress.total}`
                      : 'Zipping…'}
                  </>
                ) : (
                  <>
                    <Icon name="download" size={13} />
                    {selectedIds.size > 1 ? 'Download ZIP' : 'Download'}
                  </>
                )}
              </BBButton>
            )}
          </div>
        </div>
      )}

      {/* Context menu */}
      <ContextMenu
        open={ctxMenu.open}
        x={ctxMenu.x}
        y={ctxMenu.y}
        fileId={ctxMenu.fileId}
        fileName={ctxMenu.fileName}
        isFolder={ctxMenu.isFolder}
        mimeType={ctxMenu.mimeType}
        isStarred={ctxMenu.isStarred}
        isPinned={pinnedFolderIds.has(ctxMenu.fileId)}
        hasVersions={!ctxMenu.isFolder}
        onClose={() => setCtxMenu((prev) => ({ ...prev, open: false }))}
        onAction={(action, fileId) => {
          if (action === 'pin' || action === 'unpin') {
            handleTogglePin(fileId)
            return
          }
          if (action === 'star' || action === 'unstar') {
            onToggleStar?.(fileId)
            return
          }
          const file = files.find((f) => f.id === fileId)
          if (file) onFileAction?.(action, file)
        }}
        onColorChange={handleFolderColorChange}
      />

      {/* Share popover — anchored to the badge that opened it */}
      {sharePopover &&
        host.renderSharePopover?.({
          fileId: sharePopover.fileId,
          anchorRect: sharePopover.anchorRect,
          onClose: () => setSharePopover(null),
        })}
    </>
  )
}
