/**
 * File-type glyph shared between the Windows tray and the Activity In-Progress
 * card (LD-0105 U1 — fix 2026-11-06).
 *
 * Ported from repos/web file-icon.tsx getFileType; no @beebeeb/shared import.
 * Per-type accent colours from the shared palette (task 2320); amber is reserved
 * for encryption state only (brand rule), so these are non-amber category hues.
 */

export type FileType = 'pdf' | 'image' | 'video' | 'audio' | 'code' | 'archive' | 'default'

export function getFileType(name: string): FileType {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''

  if (ext === 'pdf') return 'pdf'

  if (
    ['jpg', 'jpeg', 'png', 'gif', 'heic', 'heif', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'tif', 'avif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'].includes(ext)
  )
    return 'image'
  if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v', 'hevc'].includes(ext)) return 'video'
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus'].includes(ext)) return 'audio'

  if (
    ['js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'cpp', 'h', 'hpp', 'cs', 'php', 'sh', 'bash', 'zsh', 'lua', 'r', 'scala', 'zig', 'asm', 'sql', 'graphql', 'proto'].includes(ext)
  )
    return 'code'
  if (['html', 'htm', 'css', 'scss', 'sass', 'less', 'vue', 'svelte', 'astro'].includes(ext)) return 'code'

  if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2', 'xz', 'zst', 'lz', 'dmg', 'iso'].includes(ext)) return 'archive'

  return 'default'
}

/** Per-type tint for the file glyph (shared palette, task 2320).
 *  These are fixed per-file-type accents — not theme tokens — which is why
 *  they are excluded from the oklch ratchet (they are not color-property
 *  assignments on a theme-aware surface). */
export const FILE_COLOR: Record<FileType, string> = {
  pdf:     '#dc2626',
  image:   '#0d9488',
  video:   '#9333ea',
  audio:   '#db2777',
  code:    '#0f766e',
  archive: '#92400e',
  default: 'var(--ink-3)',
}

export function FileGlyph({ type, size = 13, color }: { type: FileType; size?: number; color: string }) {
  const s = size
  const common = { width: s, height: s, viewBox: '0 0 16 16', fill: 'none', stroke: color, strokeWidth: 1.4 }
  switch (type) {
    case 'pdf':
      return (
        <svg {...common}>
          <path d="M4 2 L10 2 L12 4.5 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M6 8 L10 8 M6 10.3 L9 10.3" strokeLinecap="round" />
        </svg>
      )
    case 'image':
      return (
        <svg {...common}>
          <rect x="2.5" y="3" width="11" height="10" rx="1.5" />
          <circle cx="6" cy="6.5" r="1.1" fill={color} stroke="none" />
          <path d="M3 12 L6.5 8.5 L9 11 L11 9 L13.5 12" strokeLinejoin="round" />
        </svg>
      )
    case 'video':
      return (
        <svg {...common}>
          <rect x="2" y="4" width="9" height="8" rx="1.5" />
          <path d="M11 7 L14 5 L14 11 L11 9 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'audio':
      return (
        <svg {...common}>
          <path d="M6 11 L6 4 L12 2.5 L12 9.5" strokeLinejoin="round" />
          <circle cx="4.5" cy="11.5" r="1.6" />
          <circle cx="10.5" cy="9.8" r="1.6" />
        </svg>
      )
    case 'code':
      return (
        <svg {...common} strokeLinecap="round" strokeLinejoin="round">
          <path d="M6 5 L3 8 L6 11" />
          <path d="M10 5 L13 8 L10 11" />
        </svg>
      )
    case 'archive':
      return (
        <svg {...common}>
          <path d="M4 2 L12 2 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M8 2 L8 4 M8 5.5 L8 7.5" strokeLinecap="round" />
          <rect x="7" y="8" width="2" height="2.4" rx="0.4" />
        </svg>
      )
    default:
      return (
        <svg {...common}>
          <path d="M4 2 L10 2 L12 4.5 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M6 7.5 L10 7.5 M6 10 L10 10" strokeLinecap="round" />
        </svg>
      )
  }
}
