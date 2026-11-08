// Maps a file type to its icon and accent color.
import { Icon, type IconName } from '../design'

export type FileType =
  | 'folder'
  | 'pdf'
  | 'word'
  | 'excel'
  | 'powerpoint'
  | 'image'
  | 'video'
  | 'audio'
  | 'code'
  | 'archive'
  | 'data'
  | 'md'
  | 'fig'
  | 'default'

interface FileIconProps {
  type: FileType
  size?: number
}

const colorMap: Record<FileType, string> = {
  folder: 'var(--color-amber)',
  pdf: '#dc2626',
  word: '#2563eb',
  excel: '#16a34a',
  powerpoint: '#ea580c',
  image: 'var(--color-amber-deep)',
  video: '#9333ea',
  audio: '#db2777',
  code: '#0d9488',
  archive: '#92400e',
  data: 'var(--color-ink-3)',
  md: '#0f766e',
  fig: '#a855f7',
  default: 'var(--color-ink-3)',
}

const bgMap: Record<FileType, string> = {
  folder: 'var(--color-amber-bg)',
  pdf: 'var(--color-paper-2)',
  word: 'var(--color-paper-2)',
  excel: 'var(--color-paper-2)',
  powerpoint: 'var(--color-paper-2)',
  image: 'var(--color-paper-2)',
  video: 'var(--color-paper-2)',
  audio: 'var(--color-paper-2)',
  code: 'var(--color-paper-2)',
  archive: 'var(--color-paper-2)',
  data: 'var(--color-paper-2)',
  md: 'var(--color-paper-2)',
  fig: 'var(--color-paper-2)',
  default: 'var(--color-paper-2)',
}

const iconMap: Record<FileType, IconName> = {
  folder: 'folder',
  pdf: 'file-text',
  word: 'file-text',
  excel: 'file-spreadsheet',
  powerpoint: 'file-presentation',
  image: 'image',
  video: 'file-video',
  audio: 'file-audio',
  code: 'file-code',
  archive: 'file-archive',
  data: 'file-data',
  md: 'file-text',
  fig: 'file',
  default: 'file',
}

export function getFileType(name: string, isFolder: boolean): FileType {
  if (isFolder) return 'folder'
  const ext = name.split('.').pop()?.toLowerCase() ?? ''

  // Documents
  if (ext === 'pdf') return 'pdf'
  if (['doc', 'docx', 'odt', 'rtf', 'txt', 'pages'].includes(ext)) return 'word'
  if (['xls', 'xlsx', 'ods', 'numbers'].includes(ext)) return 'excel'
  if (['ppt', 'pptx', 'odp', 'key'].includes(ext)) return 'powerpoint'
  if (['md', 'mdx'].includes(ext)) return 'md'

  // Media
  if (['jpg', 'jpeg', 'png', 'gif', 'heic', 'heif', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'tif', 'avif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'].includes(ext)) return 'image'
  if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v', 'hevc'].includes(ext)) return 'video'
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus'].includes(ext)) return 'audio'

  // Code
  if (['js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'cpp', 'h', 'hpp', 'cs', 'php', 'sh', 'bash', 'zsh', 'lua', 'r', 'scala', 'zig', 'asm', 'sql', 'graphql', 'proto'].includes(ext)) return 'code'
  if (['html', 'htm', 'css', 'scss', 'sass', 'less', 'vue', 'svelte', 'astro'].includes(ext)) return 'code'

  // Archives
  if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2', 'xz', 'zst', 'lz', 'dmg', 'iso'].includes(ext)) return 'archive'

  // Data / config
  if (['json', 'csv', 'xml', 'yaml', 'yml', 'toml', 'ini', 'env', 'conf', 'cfg', 'tsv', 'ndjson', 'parquet'].includes(ext)) return 'data'

  // Design
  if (['fig', 'figma', 'sketch', 'xd', 'psd', 'ai'].includes(ext)) return 'fig'

  return 'default'
}

export function FileIcon({ type, size = 24 }: FileIconProps) {
  const iconSize = Math.round(size * 0.54)
  return (
    <div
      className="flex shrink-0 items-center justify-center rounded-md border border-line"
      style={{
        width: size,
        height: size,
        background: bgMap[type],
        color: colorMap[type],
      }}
    >
      <Icon name={iconMap[type]} size={iconSize} />
    </div>
  )
}
