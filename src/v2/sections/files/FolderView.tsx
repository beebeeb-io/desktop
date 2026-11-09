// The live Files and Backups view: reads and acts only through useDriveSource() (B01/B02); drops go through uploadDroppedPaths.
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { DriveSourceError, useDriveSource, type DriveItem, type ItemDetails, type Listing, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { EmptyFiles, GettingReadyStrip, NeedsConnection } from '../../chrome/EmptyStates'
import { backupMetaFor, uploadSummary, useLiveFolderExtras } from './liveExtras'
import { showToast } from '../../chrome/Toast'
import { uploadDroppedPaths } from '../../source/desktop'
import { hasTauri } from '../../source'
import { setSyncPaused } from '../../../desktopApi'
import { useCommand } from '../../shortcuts'
import type { Navigation } from '../../routes'
import { FilesPane, type PaneHandlers, type Renaming } from './FilesPane'
import { ConflictDialog, DetailsDialog, MoveDialog, NewFolderDialog, ProtectedAlert, ShareDialog, VersionsDialog, type FolderRow } from './Dialogs'
import { NoPreview, PreviewPanel, UploadsPanel } from './Panels'
import { copyShareLink } from './shareLink'
import { backgroundMenu, fileMenu, newMenu, sortMenu } from './menus'
import { OPEN_WITH_EDITOR, OPEN_WITH_OTHER, withOfficeOpen } from '../open/office/menu'
import { isOfficeFile } from '../open/office'
import { Menu, type MenuEntry } from './ui'
import {
  arrange, BACKUPS_ID, actionFailure, isProtected, loadPrefs, nameProblem, nameTaken, savePrefs, takenMessage, type FolderPrefs,
} from './model'

type Dialog =
  | { kind: 'share'; items: DriveItem[] }
  | { kind: 'move'; items: DriveItem[]; at: string | null; target: string | null }
  | { kind: 'new-folder'; value: string; error: string | null }
  | { kind: 'details'; item: DriveItem }
  | { kind: 'versions'; item: DriveItem; versions: VersionEntry[] }
  | { kind: 'protected'; item: DriveItem }
  | { kind: 'preview'; item: DriveItem }

interface MenuState { x: number; y: number; label: string; entries: MenuEntry[]; run: (id: string) => void }

const codeOf = (e: unknown): string => (e instanceof DriveSourceError ? e.code : 'failed')

export function FolderView({ nav, mode }: { nav: Navigation; mode: 'files' | 'backups' }) {
  const source = useDriveSource()
  const [rootId, setRootId] = useState<string | null>(mode === 'files' ? null : null)
  const folderId = mode === 'files' ? nav.route.id : nav.route.id ?? rootId
  const [listing, setListing] = useState<Listing | null>(null)
  const [prefs, setPrefs] = useState<FolderPrefs>(() => loadPrefs(folderId))
  const [selection, setSelection] = useState<ReadonlySet<string>>(new Set())
  const [anchor, setAnchor] = useState<string | null>(null)
  const [renaming, setRenaming] = useState<Renaming | null>(null)
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [details, setDetails] = useState<ItemDetails | null>(null)
  const [dropTarget, setDropTarget] = useState<string | null>(null)
  const [connectionNeeded, setConnectionNeeded] = useState<DriveItem | null>(null)
  const undo = useRef<string | null>(null)
  const live = useLiveFolderExtras()

  // Backups: the protected folder is a row of the root listing.
  useEffect(() => {
    if (mode !== 'backups') return
    source.list(null).then((l) => setRootId(l.items.find((i) => i.isFolder && i.name === 'Backups')?.id ?? null), () => setRootId(null))
  }, [source, mode])

  const load = useCallback(() => {
    if (mode === 'backups' && folderId == null) return
    source.list(folderId, { key: prefs.sort.key, dir: prefs.sort.dir }).then(setListing, () => setListing(null))
  }, [source, folderId, mode, prefs.sort.key, prefs.sort.dir])
  useEffect(() => {
    load()
    return source.subscribe((e) => { if (e.kind === 'changed' || e.kind === 'item' || e.kind === 'removed') load() })
  }, [source, load])
  useEffect(() => { setPrefs(loadPrefs(folderId)); setSelection(new Set()); setRenaming(null); setDetails(null) }, [folderId])
  // "Shows it in Files, selected" (Storage insights' largest files DW-0783#1, Search's enclosing folder): `sub=select:<id>`.
  const selectId = /^select:(.+)$/.exec(nav.route.sub ?? '')?.[1] ?? null
  useEffect(() => { if (selectId) { setSelection(new Set([selectId])); setAnchor(selectId) } }, [selectId, folderId])

  const items = useMemo(() => arrange(listing?.items ?? [], prefs.sort), [listing, prefs.sort])
  const byId = useMemo(() => new Map(items.map((i) => [i.id, i])), [items])
  const picked = useMemo(() => items.filter((i) => selection.has(i.id)), [items, selection])
  const offline = !!listing?.stale
  const path = useMemo(() => {
    const p = listing?.path ?? []
    return mode === 'backups' ? p.slice(Math.max(0, p.findIndex((c) => c.id === rootId))) : p
  }, [listing, mode, rootId])
  const shownListing = listing ? { ...listing, path } : null
  const service = path.find((c) => c.name === 'Mail Backup')
  const locked = mode === 'backups' || path.some((c) => c.id === BACKUPS_ID) || (listing ? byId.size > 0 && items.every((i) => i.readOnly) && path.length > 1 : false)
  // Offline, changes wait on this PC and upload when the connection is back (DW-1108): New stays on.
  const canNew = !locked

  const changePrefs = (next: FolderPrefs) => { setPrefs(next); savePrefs(folderId, next) }

  // Details follow the one selected row.
  const currentId = picked.length === 1 ? picked[0].id : null
  useEffect(() => {
    if (!currentId) { setDetails(null); return }
    let live = true
    source.details(currentId).then((d) => { if (live) setDetails(d) }, () => { if (live) setDetails(null) })
    return () => { live = false }
  }, [source, currentId])

  const select = (id: string, mods: { ctrl: boolean; shift: boolean }) => {
    if (mods.shift && anchor) {
      const a = items.findIndex((i) => i.id === anchor)
      const b = items.findIndex((i) => i.id === id)
      setSelection(new Set(items.slice(Math.min(a, b), Math.max(a, b) + 1).map((i) => i.id)))
      return
    }
    setAnchor(id)
    setSelection((cur) => {
      if (!mods.ctrl) return new Set([id])
      const next = new Set(cur)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const go = (id: string | null) => nav.go({ section: mode, id: mode === 'backups' && id === rootId ? null : id })
  const up = () => { const p = path; if (p.length > 1) go(p[p.length - 2].id) }

  /**
   * A failed action (DW-0109): the protected folder message for a protected item, else what failed in words, with Try
   * again repeating the action where trying again can help.
   */
  const fail = (e: unknown, item?: DriveItem, what?: { action: string; again: () => void }) => {
    if (e instanceof DriveSourceError && (e.code === 'read-only' || e.code === 'locked') && item) { setDialog({ kind: 'protected', item }); return }
    const f = actionFailure(what?.action ?? 'do that', codeOf(e))
    showToast({ kind: 'failed', text: f.text, onAction: f.retry ? what?.again : undefined })
  }

  const openInApp = (item: DriveItem) => {
    source.open(item.id).catch((e: unknown) => fail(e, item, { action: 'open it', again: () => openInApp(item) }))
  }
  const open = (item: DriveItem) => {
    if (item.isFolder) { go(item.id); return }
    if (offline && item.local === 'online-only') { setConnectionNeeded(item); return }
    if (isOfficeFile(item)) { nav.go({ section: 'open', id: item.id, sub: 'office' }); return }
    openInApp(item)
  }

  const trash = (list: DriveItem[]) => {
    const blocked = list.find(isProtected)
    if (blocked) { setDialog({ kind: 'protected', item: blocked }); return }
    if (!list.length) return
    source.moveToTrash(list.map((i) => i.id)).then(({ undoToken }) => {
      undo.current = undoToken
      setSelection(new Set())
      showToast({ kind: 'undo', text: 'Moved to Trash', onAction: () => putBack(undoToken) })
    }, (e: unknown) => fail(e, list[0], { action: 'move it to Trash', again: () => trash(list) }))
  }

  /** Undo of a move to Trash; a failure says so, and its Try again is this again. */
  const putBack = (token: string) => {
    source.undoTrash(token).catch((e: unknown) => fail(e, undefined, { action: 'put it back', again: () => putBack(token) }))
  }

  const duplicate = (list: DriveItem[]) => {
    if (!list.length) return
    source.duplicate(list.map((i) => i.id)).then((copies) => {
      setSelection(new Set(copies.map((c) => c.id)))
      showToast({ kind: 'done', text: 'Duplicated' })
    }, (e: unknown) => fail(e, list[0], { action: 'duplicate it', again: () => duplicate(list) }))
  }

  const startRename = (item: DriveItem | undefined) => {
    if (!item) return
    if (isProtected(item) || locked) { setDialog({ kind: 'protected', item }); return }
    setRenaming({ id: item.id, value: item.name, error: null })
  }
  const commitRename = () => {
    if (!renaming) return
    const item = byId.get(renaming.id)
    const value = renaming.value.trim()
    if (!item || value === item.name) { setRenaming(null); return }
    const problem = nameProblem(value)
    if (problem) { setRenaming({ ...renaming, error: problem }); return }
    if (nameTaken(value, items, item.id)) { setRenaming({ ...renaming, error: takenMessage(value, item.isFolder) }); return }
    const current = renaming
    source.rename(item.id, value).then(() => setRenaming(null), (e: unknown) => {
      if (e instanceof DriveSourceError && e.code === 'name-taken') setRenaming({ ...current, error: takenMessage(value, item.isFolder) })
      else { setRenaming(null); fail(e, item, { action: 'rename', again: () => renameTo(item, value) }) }
    })
  }
  /** Try again on a failed rename: the same name once more, without the field (DW-0109). */
  const renameTo = (item: DriveItem, value: string) => {
    source.rename(item.id, value).catch((e: unknown) => fail(e, item, { action: 'rename', again: () => renameTo(item, value) }))
  }

  const toggleOffline = (list: DriveItem[], on?: boolean) => {
    const files = list.filter((i) => !i.isFolder || i.local !== null)
    const next = on ?? !files.every((i) => i.local === 'available-offline')
    Promise.all(files.map((i) => source.setAvailableOffline(i.id, next))).then(
      () => showToast({ kind: 'done', text: next ? 'Keeping on this PC' : 'Removed from this PC' }),
      (e: unknown) => fail(e, files[0], { action: next ? 'keep it on this PC' : 'remove it from this PC', again: () => toggleOffline(list, next) }),
    )
  }

  const upload = (mode2: 'files' | 'folder') => {
    source.upload(folderId, { kind: 'pick', mode: mode2 }).then(
      ({ started }) => { if (started > 0) showToast({ kind: 'done', text: `Uploading ${started} ${started === 1 ? 'item' : 'items'}` }) },
      (e: unknown) => fail(e, undefined, { action: 'start the upload', again: () => upload(mode2) }),
    )
  }

  const reveal = (item: DriveItem) => { source.revealInFileManager(item.id).catch((e: unknown) => fail(e, item, { action: 'show it in File Explorer', again: () => reveal(item) })) }
  const showVersions = (item: DriveItem) => {
    source.versions(item.id).then((versions) => setDialog({ kind: 'versions', item, versions }), (e: unknown) => fail(e, item, { action: 'open Version history', again: () => showVersions(item) }))
  }
  const settings = (sub: string) => nav.go({ section: 'settings', sub })
  /** Restore a version (DW-0413); done closes Version history, a failure's Try again is this again. */
  const restore = (item: DriveItem, vid: string) => {
    source.restoreVersion(item.id, vid).then(
      () => { setDialog((d) => (d?.kind === 'versions' ? null : d)); showToast({ kind: 'done', text: 'Version restored' }) },
      (e: unknown) => fail(e, item, { action: 'restore it', again: () => restore(item, vid) }),
    )
  }

  const openMove = (list: DriveItem[]) => {
    const blocked = list.find(isProtected)
    if (blocked || locked) { setDialog({ kind: 'protected', item: blocked ?? list[0] }); return }
    setDialog({ kind: 'move', items: list, at: null, target: null })
  }

  const runItem = (id: string, list: DriveItem[]) => {
    const one = list[0]
    switch (id) {
      case 'open': case OPEN_WITH_EDITOR: list.forEach(open); break
      case OPEN_WITH_OTHER: if (one) void source.open(one.id); break
      case 'preview': if (one && !one.isFolder) setDialog({ kind: 'preview', item: one }); break
      case 'share': setDialog({ kind: 'share', items: list }); break
      // DW-0402 → DW-0107: one file that has a link copies that link (no choices, so Rust reuses it and never mints a
      // second one); anything else gets a new link with the board's default, 30 days (DW-0407 Rules). Rust answers.
      case 'copy-link': {
        const existing = list.length === 1 && one?.shared.kind === 'link'
        void copyShareLink(list.map((i) => i.id), { expires: existing ? '' : 'In 30 days', password: null }, showToast)
        break
      }
      case 'offline': toggleOffline(list); break
      case 'reveal': if (one) reveal(one); break
      case 'details': if (one) setDialog({ kind: 'details', item: one }); break
      case 'rename': startRename(one); break
      case 'move': openMove(list); break
      case 'duplicate': duplicate(list); break
      case 'versions': if (one) showVersions(one); break
      case 'trash': trash(list); break
      case 'delete-backup': settings('backups'); break
    }
  }

  const openMenu = (e: MouseEvent, item: DriveItem | null) => {
    if (item) {
      const list = selection.has(item.id) ? picked : [item]
      if (!selection.has(item.id)) { setSelection(new Set([item.id])); setAnchor(item.id) }
      const entries = withOfficeOpen(fileMenu(list), list)
      if (mode === 'backups' && path.length === 1 && item.name !== 'Mail Backup') entries.push({ id: 'delete-backup', label: 'Delete this backup…' })
      setMenu({ x: e.clientX, y: e.clientY, label: 'File', entries, run: (id) => runItem(id, list) })
    } else {
      setSelection(new Set())
      setMenu({
        x: e.clientX, y: e.clientY, label: 'List', entries: backgroundMenu(prefs, canNew),
        run: (id) => runBackground(id),
      })
    }
  }

  const runBackground = (id: string) => {
    if (id === 'new-folder') openNewFolder()
    else if (id === 'upload') upload('files')
    else if (id === 'view:list') changePrefs({ ...prefs, view: 'list' })
    else if (id === 'view:grid') changePrefs({ ...prefs, view: 'grid' })
    else if (id === 'folders-first') changePrefs({ ...prefs, sort: { ...prefs.sort, foldersFirst: !prefs.sort.foldersFirst } })
    else if (id === 'panel') changePrefs({ ...prefs, panel: !prefs.panel })
    else if (id.startsWith('sort:')) changePrefs({ ...prefs, sort: { ...prefs.sort, key: id.slice(5) as FolderPrefs['sort']['key'] } })
    else if (id.startsWith('order:')) changePrefs({ ...prefs, sort: { ...prefs.sort, dir: id.slice(6) as 'asc' | 'desc' } })
  }

  const openNewFolder = () => { if (canNew) setDialog({ kind: 'new-folder', value: 'Untitled folder', error: null }) }
  const createFolder = () => {
    if (dialog?.kind !== 'new-folder') return
    const name = dialog.value.trim()
    const problem = nameProblem(name) ?? (nameTaken(name, items) ? takenMessage(name, true) : null)
    if (problem) { setDialog({ ...dialog, error: problem }); return }
    source.createFolder(folderId, name).then((created) => { setDialog(null); go(created.id) }, (e: unknown) => {
      if (e instanceof DriveSourceError && e.code === 'name-taken') setDialog({ ...dialog, error: takenMessage(name, true) })
      else { setDialog(null); fail(e) }
    })
  }

  // Commands from the window's shortcuts.
  useCommand('new-folder', openNewFolder)
  useCommand('upload', useCallback(() => { if (canNew) upload('files') }, [canNew, folderId])) // eslint-disable-line react-hooks/exhaustive-deps
  useCommand('select-all', useCallback(() => setSelection(new Set(items.filter((i) => !isProtected(i)).map((i) => i.id))), [items]))
  useCommand('rename', useCallback(() => startRename(picked[0]), [picked, locked])) // eslint-disable-line react-hooks/exhaustive-deps
  useCommand('duplicate', useCallback(() => duplicate(picked), [picked])) // eslint-disable-line react-hooks/exhaustive-deps
  useCommand('trash', useCallback(() => { if (!renaming) trash(picked) }, [picked, renaming])) // eslint-disable-line react-hooks/exhaustive-deps
  useCommand('details', useCallback(() => { if (picked.length === 1) setDialog({ kind: 'details', item: picked[0] }) }, [picked]))
  useCommand('toggle-details-panel', useCallback(() => changePrefs({ ...prefs, panel: !prefs.panel }), [prefs])) // eslint-disable-line react-hooks/exhaustive-deps
  useCommand('undo', useCallback(() => {
    const token = undo.current
    if (!token) return
    undo.current = null
    source.undoTrash(token).catch((e: unknown) => fail(e))
  }, [source]))

  // Row keys: arrows move the selection, Ctrl+Down opens, Ctrl+Up goes up, Space previews, Esc ends the selection.
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement
      // A key the list already used (a space inside a name being typed) is not the window's.
      if (e.defaultPrevented || dialog || menu || t.tagName === 'INPUT' || t.tagName === 'SELECT') return
      if (e.key === 'ArrowDown' && e.ctrlKey) { e.preventDefault(); picked.forEach(open) }
      else if (e.key === 'ArrowUp' && e.ctrlKey) { e.preventDefault(); up() }
      else if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && !e.altKey && !e.ctrlKey) {
        e.preventDefault()
        const at = items.findIndex((i) => i.id === (picked[picked.length - 1]?.id ?? ''))
        const next = items[Math.max(0, Math.min(items.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))]
        if (next) select(next.id, { ctrl: false, shift: e.shiftKey })
      } else if (e.key === ' ' && picked.length === 1 && !picked[0].isFolder) { e.preventDefault(); setDialog({ kind: 'preview', item: picked[0] }) }
      else if (e.key === 'Escape') { setSelection(new Set()); setConnectionNeeded(null) }
      else if (e.key === 'Enter' && picked.length === 1 && !e.altKey) open(picked[0])
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  })

  // Drops from File Explorer: the window's native drag-drop event carries the paths (a browser File has none).
  useEffect(() => {
    if (!hasTauri()) return
    let off: (() => void) | undefined
    let gone = false
    const hit = (pos: { x: number; y: number }): DriveItem | undefined => {
      const dpr = window.devicePixelRatio || 1
      const row = document.elementFromPoint(pos.x / dpr, pos.y / dpr)?.closest('[data-row],[data-tile]')
      const id = row?.getAttribute('data-row') ?? row?.getAttribute('data-tile')
      return id ? byId.get(id) : undefined
    }
    void import('@tauri-apps/api/webview').then(({ getCurrentWebview }) => getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload
      if (p.type === 'leave') { setDropTarget(null); return }
      const over = hit(p.position)
      const target = over?.isFolder && !isProtected(over) ? over : undefined
      if (p.type === 'over' || p.type === 'enter') { setDropTarget(target?.id ?? null); return }
      setDropTarget(null)
      if (over && !target) { setDialog({ kind: 'protected', item: over }); return }
      if (locked) { setDialog({ kind: 'protected', item: items[0] }); return }
      void uploadDroppedPaths(target?.id ?? folderId, p.paths).then(
        ({ started }) => showToast({ kind: 'done', text: `Uploading ${started} ${started === 1 ? 'item' : 'items'}` }),
        (e: unknown) => showToast({ kind: 'failed', text: actionFailure('upload them', codeOf(e)).text }),
      )
    })).then((fn) => { if (gone) fn(); else off = fn })
    return () => { gone = true; off?.() }
  }, [byId, folderId, locked, items])

  const dropRows = (folder: DriveItem, e: React.DragEvent) => {
    const raw = e.dataTransfer.getData('application/x-bb-ids')
    if (!raw) return
    const ids = (JSON.parse(raw) as string[]).filter((id) => id !== folder.id)
    source.move(ids, folder.id).then(() => showToast({ kind: 'done', text: `Moved to ${folder.name}` }), (e2: unknown) => fail(e2, folder))
  }

  /** DW-0121's Try again: checks the connection; back, the file opens; still offline, the panel stays and the list re-reads. */
  const tryConnection = (item: DriveItem) => {
    source.sync().then((s) => {
      load()
      if (s.phase === 'offline') return
      setConnectionNeeded(null)
      openInApp(item)
    }, () => load())
  }

  /** DW-0414: New's menu, under the control that opened it (New in the toolbar, or Add files in an empty folder). */
  const openNewMenu = (e: MouseEvent, align: 'right' | 'center') => {
    e.stopPropagation()
    const r = e.currentTarget.getBoundingClientRect()
    setMenu({
      x: align === 'right' ? r.right - 264 : r.left + r.width / 2 - 132, y: r.bottom + 4, label: 'New', entries: newMenu(),
      run: (id) => {
        if (id === 'new-folder') openNewFolder()
        else if (id === 'upload') upload('files')
        else if (id === 'upload-folder') upload('folder')
        else if (id === 'new-file') nav.go({ section: 'open', id: nav.route.id, sub: 'new-file' })
        else if (id === 'new-document') nav.go({ section: 'open', id: nav.route.id, sub: 'new-document' })
      },
    })
  }

  const handlers: PaneHandlers = {
    onSelect: select,
    onClear: () => setSelection(new Set()),
    onOpen: open,
    onContext: openMenu,
    onBack: () => nav.back(),
    onForward: () => nav.forward(),
    onCrumb: go,
    onView: (view) => changePrefs({ ...prefs, view }),
    onSortColumn: (key) => changePrefs({ ...prefs, sort: { ...prefs.sort, key, dir: prefs.sort.key === key && prefs.sort.dir === 'asc' ? 'desc' : 'asc' } }),
    // The view's own click closes a menu, so the click that opens one stops here (M1 audit: New and Sort never opened).
    onSortMenu: (e) => { e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); setMenu({ x: r.left, y: r.bottom + 4, label: 'Sort', entries: sortMenu(prefs), run: runBackground }) },
    onNewMenu: (e) => openNewMenu(e, 'right'),
    onTogglePanel: () => changePrefs({ ...prefs, panel: !prefs.panel }),
    onFind: () => nav.go({ section: 'search' }),
    onRenameChange: (value) => setRenaming((r) => (r ? { ...r, value, error: null } : r)),
    onRenameCommit: commitRename,
    onRenameCancel: () => setRenaming(null),
    onToggleOffline: (item, on) => toggleOffline([item], on),
    onBulk: (a) => {
      if (a === 'done') setSelection(new Set())
      else if (a === 'select-all') setSelection(new Set(items.filter((i) => !isProtected(i)).map((i) => i.id)))
      else if (a === 'share') setDialog({ kind: 'share', items: picked })
      else if (a === 'offline') toggleOffline(picked)
      else if (a === 'move') openMove(picked)
      else if (a === 'trash') trash(picked)
    },
    onShare: (item) => setDialog({ kind: 'share', items: [item] }),
    onVersions: showVersions,
    onDropOnFolder: dropRows,
  }

  const top = service ? (
    <div role="status" data-banner="managed" className="flex min-h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 py-2 text-[13px] leading-5 text-ink-2">
      <span className="min-w-0 flex-1"><span className="font-medium text-ink">{service.name} manages this folder.</span> You can open and download its files.</span>
      <button type="button" onClick={() => settings('mail-backup')} className="h-7 flex-none rounded-md border border-line bg-paper px-3 text-[13px] font-medium text-ink hover:bg-paper-3 focus-visible:outline-2 focus-visible:outline-ink-3">Settings</button>
    </div>
  ) : live.status?.listing && mode === 'files' ? (
    <GettingReadyStrip loading={{ done: live.status.listing.done, total: live.status.listing.total, thumbnails: 'waiting' }} />
  ) : null
  const conflict = live.conflicts[0]

  const emptyAndAddable = items.length === 0 && listing && canNew
  return (
    <div role="presentation" className="relative flex min-h-0 flex-1" onClick={() => setMenu(null)}>
      <FilesPane
        p={{
          listing: shownListing, items, prefs, selection, currentId, renaming, offline, dropTarget, details,
          columns: mode === 'backups' && path.length === 1 ? 'backups' : 'files', canNew, top,
          title: mode === 'backups' ? 'Backups' : 'Files', handlers,
          transfers: live.transfers, backupMeta: backupMetaFor(live.status, items),
          empty: emptyAndAddable ? <EmptyFiles onAddFiles={(e) => openNewMenu(e, 'center')} /> : undefined,
        }}
      />
      {connectionNeeded && <NeedsConnection fileName={connectionNeeded.name} onTryAgain={() => tryConnection(connectionNeeded)} onClose={() => setConnectionNeeded(null)} />}
      {menu && <Menu x={menu.x} y={menu.y} label={menu.label} entries={menu.entries} onPick={menu.run} onClose={() => setMenu(null)} />}
      {live.uploadsOpen && (
        <UploadsPanel
          rows={live.uploads} summary={uploadSummary(live.uploads)} onTryAgain={live.retry} onClose={live.closeUploads}
          onPauseAll={() => { void setSyncPaused(true).then((r) => showToast(r.ok ? { kind: 'done', text: 'Uploads paused' } : { kind: 'failed', text: 'Couldn’t pause uploads' })) }}
        />
      )}
      {conflict && !dialog && (
        <ConflictDialog
          index={1} total={live.conflicts.length} name={conflict.name}
          mine={{ label: 'This PC', keep: 'this PC’s' }} theirs={{ label: 'In Beebeeb', keep: 'Beebeeb’s' }}
          onKeep={(choice) => live.resolve(conflict.id, choice)} onLater={() => live.later(conflict.id)}
        />
      )}
      {dialog?.kind === 'share' && (
        <ShareDialog
          name={dialog.items[0].name} size={dialog.items[0].size} count={dialog.items.length}
          onCopy={(o) => { const items = dialog.items; setDialog(null); void copyShareLink(items.map((i) => i.id), o, showToast) }}
          onInvite={() => { setDialog(null); nav.go({ section: 'shared' }) }} onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === 'move' && <MoveHost dialog={dialog} setDialog={setDialog} onDone={() => setDialog((d) => (d?.kind === 'move' ? null : d))} fail={fail} />}
      {dialog?.kind === 'new-folder' && (
        <NewFolderDialog parentName={listing?.folder.name ?? 'Files'} value={dialog.value} error={dialog.error} onChange={(value) => setDialog({ ...dialog, value, error: null })} onCreate={createFolder} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === 'details' && <DetailsDialog item={dialog.item} details={details?.item.id === dialog.item.id ? details : null} onVersions={() => showVersions(dialog.item)} onClose={() => setDialog(null)} />}
      {dialog?.kind === 'versions' && (
        <VersionsDialog
          name={dialog.item.name} versions={dialog.versions} onClose={() => setDialog(null)}
          // DW-0413: the current version previews as the file; an earlier one has no read of its own yet (no versioned
          // read in DriveSource), so it says so instead of showing the current content under an older date.
          onOpen={(vid) => {
            if (dialog.versions.find((v) => v.id === vid)?.current) setDialog({ kind: 'preview', item: dialog.item })
            else showToast({ kind: 'failed', text: 'An earlier version can’t be previewed in this build yet. Restore it to open it.' })
          }}
          onRestore={(vid) => restore(dialog.item, vid)}
        />
      )}
      {dialog?.kind === 'protected' && <ProtectedAlert name={dialog.item.name} onOk={() => setDialog(null)} onSettings={() => { setDialog(null); settings('mail-backup') }} />}
      {dialog?.kind === 'preview' && <PreviewHost item={dialog.item} items={items} onClose={() => setDialog(null)} onSwitch={(item) => setDialog({ kind: 'preview', item })} onOpen={open} onShare={(item) => setDialog({ kind: 'share', items: [item] })} onKeep={(item) => toggleOffline([item], true)} />}
    </div>
  )
}

function MoveHost({ dialog, setDialog, onDone, fail }: { dialog: Extract<Dialog, { kind: 'move' }>; setDialog: (d: Dialog) => void; onDone: () => void; fail: (e: unknown, item?: DriveItem, what?: { action: string; again: () => void }) => void }) {
  const source = useDriveSource()
  const [listing, setListing] = useState<Listing | null>(null)
  // DW-0409's New folder asks for the name first (DW-0417), then the Move dialog comes back with the folder selected.
  const [naming, setNaming] = useState<{ value: string; error: string | null } | null>(null)
  useEffect(() => { source.list(dialog.at).then(setListing, () => setListing(null)) }, [source, dialog.at])
  const moving = new Set(dialog.items.map((i) => i.id))
  // Backups and Trash aren't offered; folders a service manages can't be chosen (DW-0409).
  const folders: FolderRow[] = (listing?.items ?? []).filter((i) => i.isFolder && !moving.has(i.id) && !isProtected(i)).map((i) => ({ id: i.id, name: i.name, count: i.childCount ?? 0 }))
  const target = dialog.target ? folders.find((f) => f.id === dialog.target) : undefined
  const here = listing?.folder
  // The move; done closes the dialog with "Moved to …", a failure's Try again is this again (dialog open or not).
  const moveThere = (items: DriveItem[], to: string | null, name: string) => {
    source.move(items.map((i) => i.id), to).then(
      () => { onDone(); showToast({ kind: 'done', text: `Moved to ${name}` }) },
      (e: unknown) => fail(e, items[0], { action: 'move it', again: () => moveThere(items, to, name) }),
    )
  }
  if (naming) {
    const create = () => {
      const name = naming.value.trim()
      const problem = nameProblem(name) ?? (nameTaken(name, listing?.items ?? []) ? takenMessage(name, true) : null)
      if (problem) { setNaming({ ...naming, error: problem }); return }
      source.createFolder(dialog.at, name).then((c) => {
        setNaming(null)
        setDialog({ ...dialog, target: c.id })
        void source.list(dialog.at).then(setListing, () => setListing(null))
      }, (e: unknown) => {
        if (e instanceof DriveSourceError && e.code === 'name-taken') setNaming({ ...naming, error: takenMessage(name, true) })
        else { setNaming(null); fail(e, undefined, { action: 'make the folder', again: () => setNaming({ value: name, error: null }) }) }
      })
    }
    return <NewFolderDialog parentName={here?.name ?? 'Files'} value={naming.value} error={naming.error} onChange={(value) => setNaming({ value, error: null })} onCreate={create} onClose={() => setNaming(null)} />
  }
  return (
    <MoveDialog
      path={listing?.path ?? [{ id: null, name: 'Files' }]} folders={folders} selectedId={dialog.target} targetName={target?.name ?? here?.name ?? 'Files'}
      onSelect={(id) => setDialog({ ...dialog, target: id })} onEnter={(id) => setDialog({ ...dialog, at: id, target: null })} onCrumb={(id) => setDialog({ ...dialog, at: id, target: null })}
      onNewFolder={() => setNaming({ value: 'Untitled folder', error: null })}
      onMove={() => moveThere(dialog.items, dialog.target ?? dialog.at, target?.name ?? here?.name ?? 'Files')}
      onClose={onDone}
    />
  )
}

function PreviewHost({ item, items, onClose, onSwitch, onOpen, onShare, onKeep }: {
  item: DriveItem; items: DriveItem[]; onClose: () => void; onSwitch: (i: DriveItem) => void; onOpen: (i: DriveItem) => void; onShare: (i: DriveItem) => void; onKeep: (i: DriveItem) => void
}) {
  const source = useDriveSource()
  const [body, setBody] = useState<{ kind: 'text'; text: string } | { kind: 'image'; url: string } | { kind: 'none' } | null>(null)
  useEffect(() => {
    let live = true
    setBody(null)
    const isText = /^text\//.test(item.mime ?? '') || /\.(md|txt|json|csv)$/i.test(item.name)
    if (isText) source.readText(item.id).then((d) => { if (live) setBody({ kind: 'text', text: d.text }) }, () => { if (live) setBody({ kind: 'none' }) })
    else if (item.hasThumbnail) source.thumbnail(item.id, 'large').then((u) => { if (live) setBody(u ? { kind: 'image', url: u } : { kind: 'none' }) }, () => { if (live) setBody({ kind: 'none' }) })
    else setBody({ kind: 'none' })
    return () => { live = false }
  }, [source, item])
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const at = items.findIndex((i) => i.id === item.id)
      const step = (d: number) => { for (let i = at + d; i >= 0 && i < items.length; i += d) if (!items[i].isFolder) { onSwitch(items[i]); return } }
      // Space or Esc closes Preview (DW-0408, DW-0408b).
      if (e.key === ' ' || e.key === 'Escape') { e.preventDefault(); onClose() }
      else if (e.key === 'ArrowLeft') step(-1)
      else if (e.key === 'ArrowRight') step(1)
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [item, items, onClose, onSwitch])
  if (!body) return null
  if (body.kind === 'none') return <NoPreview name={item.name} size={item.size ?? 0} onOpenElsewhere={() => onOpen(item)} onKeep={() => { onKeep(item); onClose() }} onClose={onClose} />
  return (
    <PreviewPanel name={item.name} onClose={onClose} onOpenInApp={() => onOpen(item)} onShare={() => onShare(item)}>
      {body.kind === 'text'
        ? <pre className="m-0 w-full whitespace-pre-wrap break-words rounded-sm border border-line bg-white p-6 font-mono text-[12px] leading-5 text-ink">{body.text}</pre>
        : <img src={body.url} alt={item.name} className="max-h-full max-w-full rounded-sm object-contain" />}
    </PreviewPanel>
  )
}
