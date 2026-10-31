/**
 * Trash's pure decisions (task 1683 slice 4; spec §7.7 and hi-fi section 3): the words the Windows shell's Trash has,
 * unchanged, the row's second line, and when a file was deleted. `TrashPane.tsx` only draws them.
 */
import type { DesktopTrashItem } from './desktopApi'
import { formatStorageSize } from './storageFormat'
import { dayLabel, parseWhen } from './whenFormat'

export const TRASH_INTRO = 'Deleted files in your vault. Restore puts them back through sync; permanent delete removes them from Trash.'
export const TRASH_EMPTY_TITLE = 'Trash is empty'
export const TRASH_EMPTY_HINT = 'Deleted files will appear here.'

/** "today", "yesterday", "12 Oct", "3 Mar 2025"; null when the server's time cannot be read (no dash, no guess). */
export function trashDeletedWhen(updatedAt: string, now: Date): string | null {
  const date = parseWhen(updatedAt)
  return date === null ? null : dayLabel(date, now)
}

/**
 * "File · 48 KB · deleted yesterday". A folder shows a size only when the server sent one: it sends 0 for folders, and
 * "0 B" would be a claim about what is inside.
 */
export function trashRowLine(item: DesktopTrashItem, now: Date, locale?: string): string {
  const size = item.is_folder && !(item.size_bytes > 0) ? null : formatStorageSize(item.size_bytes, locale)
  const when = trashDeletedWhen(item.updated_at, now)
  return [item.is_folder ? 'Folder' : 'File', size, when === null ? null : `deleted ${when}`]
    .filter((part): part is string => part !== null)
    .join(' · ')
}

export function trashDeleteSheetText(name: string): string {
  return `${name} will be removed from Trash. This cannot be undone.`
}
