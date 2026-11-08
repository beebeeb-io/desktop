// The shared file-row container plus the header/status bar class constants.
import type { HTMLAttributes } from 'react'

/**
 * Task 2237b — the Files presentation primitives, shared by the owner's Files
 * list and the shared-folder guest page (Guus, 2026-10-28: the shared folder
 * "has an alike folder and file overview as files has right now").
 *
 * Each browser supplies its own columns and actions; the row surface, the
 * column-header bar, the header label type and the footer status bar are one
 * definition so the two views cannot drift apart.
 */

/** Shared Files row surface. */
export function FileBrowserRow({ className = '', ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div {...props} className={`file-row group grid min-h-11 cursor-pointer gap-2.5 transition-colors duration-150 md:gap-[14px] ${className}`} />
}

/** The bar above the rows that holds the column labels. */
export const FILE_HEADER_BAR = 'px-3 md:px-5 py-2 border-b border-line bg-paper-2 grid gap-2.5 md:gap-[14px]'

/** A column label ("Name", "Size", "Modified"). */
export const FILE_HEADER_LABEL = 'text-[10px] font-medium uppercase tracking-wider text-ink-3'

/** The bar under the rows: item count and the encryption status line. */
export const FILE_STATUS_BAR = 'px-3 md:px-5 py-2 border-t border-line bg-paper-2 flex items-center gap-2 md:gap-3.5 text-[11px] text-ink-3'

/** Row padding, as in Files. */
export const FILE_ROW_PADDING = 'px-3 md:px-5 py-[11px]'
