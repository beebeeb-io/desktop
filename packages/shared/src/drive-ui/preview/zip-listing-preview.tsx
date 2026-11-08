// The zip archive listing preview.
import { useEffect, useState } from 'react'
import { Icon, BBButton } from '../../design'
import { readZipListing, type ZipListing } from '../lib/zip-listing'
import { formatBinarySize as formatSize } from '../lib/format'
import { UnsupportedPreview } from './unsupported-preview'

interface ZipListingPreviewProps {
  blob: Blob
  filename: string
}

/**
 * A honest file LISTING for `.zip` archives — names, sizes, folders — read
 * from the archive's own Central Directory (see `lib/zip-listing.ts`). No
 * entry is ever extracted or decompressed; this is a directory read, not a
 * preview of any one file inside the archive (real gap vs. actually opening
 * a member — flagged as a known limitation, not fixed here — see this
 * task's Notes).
 */
export function ZipListingPreview({ blob, filename }: ZipListingPreviewProps) {
  const [listing, setListing] = useState<ZipListing | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    setListing(null)
    setFailed(false)

    async function load() {
      try {
        const result = await readZipListing(blob)
        if (cancelled) return
        if (result) setListing(result)
        else setFailed(true)
      } catch {
        if (!cancelled) setFailed(true)
      }
    }

    load()
    return () => {
      cancelled = true
    }
  }, [blob])

  if (failed) {
    return <UnsupportedPreview blob={blob} filename={filename} />
  }

  if (!listing) {
    return (
      <div className="flex flex-col items-center gap-3">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-line border-t-amber" />
        <span className="text-sm text-ink-3">Reading archive...</span>
      </div>
    )
  }

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
    <div data-testid="zip-listing" className="flex h-full w-full flex-col gap-3 overflow-hidden px-4 py-4">
      <div className="flex shrink-0 items-center justify-between">
        <div className="flex items-center gap-2 text-sm text-ink-2">
          <Icon name="folder" size={16} />
          <span>
            {listing.totalEntries} {listing.totalEntries === 1 ? 'item' : 'items'}
            {listing.truncated ? ` — showing first ${listing.entries.length}` : ''}
          </span>
        </div>
        <BBButton variant="amber" size="sm" onClick={handleDownload}>
          <Icon name="download" size={14} className="mr-2" />
          Download
        </BBButton>
      </div>
      <div className="min-h-0 flex-1 overflow-auto rounded-md border border-line">
        <table className="w-full border-collapse text-left text-xs">
          <thead className="sticky top-0 bg-paper">
            <tr className="border-b border-line text-ink-3">
              <th className="px-3 py-2 font-medium">Name</th>
              <th className="px-3 py-2 font-medium">Size</th>
            </tr>
          </thead>
          <tbody>
            {listing.entries.map((entry, i) => (
              <tr key={`${entry.name}-${i}`} className="border-b border-line last:border-b-0">
                <td className="px-3 py-1.5 font-mono text-[11px] text-ink break-all">
                  {entry.isDirectory ? <Icon name="folder" size={12} className="mr-1.5 inline text-ink-3" /> : null}
                  {entry.name}
                </td>
                <td className="whitespace-nowrap px-3 py-1.5 font-mono text-[11px] text-ink-3">
                  {entry.isDirectory ? '—' : formatSize(entry.uncompressedSize)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {listing.truncated && (
        <p className="shrink-0 text-center text-[11px] text-ink-4">
          Listing truncated — download the archive to see every file.
        </p>
      )}
    </div>
  )
}
