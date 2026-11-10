// The board's menus, in its words and order (DW-0402, DW-0404, DW-0414, DW-1103). Pure: they only build entries.
import type { DriveItem } from '@beebeeb/shared/drive-ui'
import type { MenuEntry } from './ui'
import { isProtected, orderLabels, SORT_LABEL, type FolderPrefs } from './model'
import type { SortKey } from '@beebeeb/shared/drive-ui'

/** What the host knows about the item that the row itself does not carry. */
export interface FileMenuOptions {
  /** Shared with you: the owner's name is not yours to change, so Rename is left out. */
  sharedWithMe?: boolean
  /** The link command can make a link for this item; false leaves Copy link out (a folder it refuses). */
  linkShare?: boolean
}

/** DW-0402: the file menu. One item, many items, a folder, and a protected folder each leave out what they must. */
export function fileMenu(items: DriveItem[], options: FileMenuOptions = {}): MenuEntry[] {
  const many = items.length > 1
  const one = items[0]
  const locked = items.some(isProtected)
  const linkShare = options.linkShare ?? true
  if (locked) {
    return [
      { id: 'open', label: 'Open', shortcut: 'Ctrl+↓' },
      { id: 'preview', label: 'Preview', shortcut: 'Space', disabled: one?.isFolder },
      { id: 'reveal', label: 'Show in File Explorer' },
      { id: 'details', label: 'Details', shortcut: 'Alt+Enter' },
    ]
  }
  const keeping = items.every((i) => i.local === 'available-offline')
  const e: MenuEntry[] = [
    { id: 'open', label: 'Open', shortcut: 'Ctrl+↓' },
    { id: 'preview', label: 'Preview', shortcut: 'Space', disabled: items.some((i) => i.isFolder) },
    { id: 'share', label: 'Share…' },
    ...(linkShare ? [{ id: 'copy-link', label: 'Copy link' }] : []),
    { id: 'offline', label: keeping ? 'Remove from this PC' : 'Make available offline' },
    { id: 'reveal', label: 'Show in File Explorer' },
  ]
  if (!many) e.push({ id: 'details', label: 'Details', shortcut: 'Alt+Enter' })
  e[e.length - 1].dividerAfter = true
  e.push({ id: 'move', label: 'Move…' }, { id: 'duplicate', label: 'Duplicate' })
  if (!many) {
    if (!options.sharedWithMe) e.splice(e.findIndex((x) => x.id === 'move'), 0, { id: 'rename', label: 'Rename', shortcut: 'F2' })
    e.push({ id: 'versions', label: 'Version history…', disabled: !!one?.isFolder })
  }
  e[e.length - 1].dividerAfter = true
  e.push({ id: 'trash', label: 'Move to Trash', shortcut: 'Delete' })
  return e
}

/** DW-0404 as written: "Sort by Name ✓ …; Order A to Z ✓ …; Folders first ✓". */
export function sortMenu(prefs: FolderPrefs): MenuEntry[] {
  const [asc, desc] = orderLabels(prefs.sort.key)
  return [
    ...(Object.keys(SORT_LABEL) as SortKey[]).map((k, i) => ({ id: `sort:${k}`, label: SORT_LABEL[k], checked: prefs.sort.key === k, dividerAfter: i === 3 })),
    { id: 'order:asc', label: asc, checked: prefs.sort.dir === 'asc' },
    { id: 'order:desc', label: desc, checked: prefs.sort.dir === 'desc', dividerAfter: true },
    { id: 'folders-first', label: 'Folders first', checked: prefs.sort.foldersFirst },
  ]
}

/** DW-1103: right-click on the list's background. */
export function backgroundMenu(prefs: FolderPrefs, canNew: boolean, sortOpen = false): MenuEntry[] {
  return [
    { id: 'new-folder', label: 'New folder', shortcut: 'Ctrl+Shift+N', disabled: !canNew },
    { id: 'upload', label: 'Upload files…', shortcut: 'Ctrl+O', disabled: !canNew, dividerAfter: true },
    { id: 'view:list', label: 'as List', checked: prefs.view === 'list' },
    { id: 'view:grid', label: 'as Grid', checked: prefs.view === 'grid' },
    { id: 'sort', label: 'Sort by', submenu: sortMenu(prefs).filter((e) => e.id !== 'folders-first'), open: sortOpen },
    { id: 'folders-first', label: 'Folders first', checked: prefs.sort.foldersFirst, dividerAfter: true },
    { id: 'panel', label: 'Show details panel', shortcut: 'Alt+Shift+P', checked: prefs.panel },
  ]
}

/** DW-0414: New. */
export function newMenu(): MenuEntry[] {
  return [
    { id: 'new-folder', label: 'New folder', shortcut: 'Ctrl+Shift+N' },
    { id: 'new-file', label: 'New file…', shortcut: 'Ctrl+N' },
    { id: 'new-document', label: 'New document…', dividerAfter: true },
    { id: 'upload', label: 'Upload files…', shortcut: 'Ctrl+O' },
    { id: 'upload-folder', label: 'Upload a folder…' },
  ]
}
