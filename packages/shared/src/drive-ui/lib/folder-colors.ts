// ─── Folder color labels ─────────────────────────────────────────────────────
// Colors are stored in localStorage per folder ID.
// Tailwind cannot generate dynamic class names at runtime (purges unused),
// so all color rendering uses inline CSS via `dot` hex values.

export interface FolderColorOption {
  name: string
  dot: string
}

export const FOLDER_COLORS: FolderColorOption[] = [
  { name: 'red',    dot: '#ef4444' },
  { name: 'orange', dot: '#f97316' },
  { name: 'amber',  dot: '#f59e0b' },
  { name: 'green',  dot: '#22c55e' },
  { name: 'teal',   dot: '#14b8a6' },
  { name: 'blue',   dot: '#3b82f6' },
  { name: 'purple', dot: '#a855f7' },
  { name: 'pink',   dot: '#ec4899' },
]

const FOLDER_COLOR_KEY = (folderId: string) => `beebeeb.folder-color.${folderId}`

export function getFolderColor(folderId: string): string | null {
  try {
    return localStorage.getItem(FOLDER_COLOR_KEY(folderId))
  } catch {
    return null
  }
}

export function setFolderColor(folderId: string, color: string | null) {
  try {
    if (color) {
      localStorage.setItem(FOLDER_COLOR_KEY(folderId), color)
    } else {
      localStorage.removeItem(FOLDER_COLOR_KEY(folderId))
    }
  } catch { /* ignore */ }
}

/** Returns the hex dot color for a stored color name, or null. */
export function getFolderColorDot(folderId: string): string | null {
  const name = getFolderColor(folderId)
  if (!name) return null
  return FOLDER_COLORS.find((c) => c.name === name)?.dot ?? null
}
