/**
 * Unsupported file preview — shown for file types that cannot be rendered
 * in-browser. Displays file metadata and a prominent download button.
 *
 * Used for: camera RAW (DNG, CR2, CR3), PPTX, and any other binary format
 * without a dedicated viewer.
 */

import { Icon } from '../../design'
import { BBButton } from '../../design'
import { FileIcon, getFileType } from '../file-icon'
import { formatBinarySize as formatSize } from '../lib/format'

interface UnsupportedPreviewProps {
  blob: Blob
  filename: string
  /** Optional thumbnail URL (e.g. from server-generated thumbnail for RAW files) */
  thumbnailUrl?: string | null
}

function getExtension(filename: string): string {
  const dot = filename.lastIndexOf('.')
  if (dot === -1) return ''
  return filename.slice(dot + 1).toLowerCase()
}

function getMimeLabel(filename: string): string {
  const ext = getExtension(filename)
  const labels: Record<string, string> = {
    dng: 'Adobe DNG (RAW)',
    cr2: 'Canon CR2 (RAW)',
    cr3: 'Canon CR3 (RAW)',
    nef: 'Nikon NEF (RAW)',
    arw: 'Sony ARW (RAW)',
    orf: 'Olympus ORF (RAW)',
    rw2: 'Panasonic RW2 (RAW)',
    raf: 'Fujifilm RAF (RAW)',
    pptx: 'PowerPoint Presentation',
    ppt: 'PowerPoint Presentation',
    odp: 'OpenDocument Presentation',
    key: 'Keynote Presentation',
  }
  return labels[ext] ?? `${ext.toUpperCase()} File`
}

export function UnsupportedPreview({ blob, filename, thumbnailUrl }: UnsupportedPreviewProps) {
  const ext = getExtension(filename)
  const fileType = getFileType(filename, false)
  const sizeStr = formatSize(blob.size)
  const mimeLabel = getMimeLabel(filename)

  function handleDownload() {
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
  }

  return (
    <div className="flex flex-col items-center gap-6 text-center py-8 px-6 max-w-[24rem] mx-auto">
      {/* Thumbnail preview if available */}
      {thumbnailUrl ? (
        <div className="relative">
          <img
            src={thumbnailUrl}
            alt=""
            className="max-h-48 rounded-lg shadow-2 object-contain"
            draggable={false}
          />
          <div className="absolute bottom-2 right-2 rounded-md bg-paper/90 px-2 py-0.5 text-[10px] font-mono text-ink-3 shadow-1 backdrop-blur-sm uppercase">
            {ext}
          </div>
        </div>
      ) : (
        <FileIcon type={fileType} size={56} />
      )}

      {/* File info */}
      <div className="flex flex-col gap-1.5">
        <p className="text-sm font-medium text-ink break-all leading-snug">
          {filename}
        </p>
        <p className="text-xs text-ink-3">
          {mimeLabel}
        </p>
        <p className="text-xs font-mono text-ink-4">
          {sizeStr}
        </p>
      </div>

      {/* Message */}
      <p className="text-sm text-ink-2">
        Preview not available for this file type.
        {thumbnailUrl ? ' Showing thumbnail.' : ''} Download to view full quality.
      </p>

      {/* Download button */}
      <BBButton variant="amber" size="md" onClick={handleDownload}>
        <Icon name="download" size={14} className="mr-2" />
        Download
      </BBButton>
    </div>
  )
}
