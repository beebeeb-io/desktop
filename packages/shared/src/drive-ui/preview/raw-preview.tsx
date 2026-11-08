// The fallback preview for unknown binary types.
import { useEffect, useState } from 'react'
import { previewDecoders } from '../lib/preview-decoders'
import type { RawExifInfo } from '../lib/raw-embedded-jpeg'
import { ImagePreview } from './image-preview'
import { UnsupportedPreview } from './unsupported-preview'

interface RawPreviewProps {
  blob: Blob
  filename: string
  zoom?: number
  rotation?: number
  onZoomChange?: (z: number) => void
  onClose?: () => void
  onPrev?: () => void
  onNext?: () => void
  hasPrev?: boolean
  hasNext?: boolean
  /** Reports the extracted EXIF summary (camera/lens/exposure) once
   *  resolved, or `null` on failure/no-EXIF — lets FilePreview's Info rail
   *  show camera details for RAW files, same idea as mobile's Info sheet
   *  (task 1569). Called at most once per `blob`. */
  onInfo?: (info: RawExifInfo | null) => void
}

export function RawPreview({
  blob,
  filename,
  zoom = 1,
  rotation = 0,
  onZoomChange,
  onClose,
  onPrev,
  onNext,
  hasPrev,
  hasNext,
  onInfo,
}: RawPreviewProps) {
  const [previewBlob, setPreviewBlob] = useState<Blob | null>(null)
  const [loading, setLoading] = useState(true)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setFailed(false)
    setPreviewBlob(null)

    async function extract() {
      try {
        const result = await previewDecoders().extractRawPreview(blob)
        if (cancelled) return
        onInfo?.(result.exif)
        if (result.previewBlob) {
          setPreviewBlob(result.previewBlob)
        } else {
          setFailed(true)
        }
      } catch {
        if (!cancelled) {
          onInfo?.(null)
          setFailed(true)
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    extract()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- onInfo is a
    // per-render callback prop; re-running extraction on its identity
    // changing would re-extract on every parent render, not just on a new
    // file.
  }, [blob])

  if (loading) {
    return (
      <div className="flex flex-col items-center gap-3">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-line border-t-amber" />
        <span className="text-sm text-ink-3">Extracting preview...</span>
      </div>
    )
  }

  if (failed || !previewBlob) {
    return <UnsupportedPreview blob={blob} filename={filename} />
  }

  // Delegate actual rendering (zoom/rotation/prev-next/swipe gestures) to
  // ImagePreview — same pattern as HeicPreview's post-decode handoff, so RAW
  // gets the identical, already-battle-tested image chrome instead of a
  // second hand-maintained copy of it. The "embedded preview" caption rides
  // alongside it as a sibling overlay (ImagePreview itself stays generic —
  // HeicPreview's successful decode gets no such caption because a HEIC
  // conversion IS full quality; RAW's extracted JPEG is not).
  return (
    <div className="relative flex h-full w-full items-center justify-center overflow-hidden">
      <ImagePreview
        blob={previewBlob}
        filename={filename}
        zoom={zoom}
        rotation={rotation}
        onZoomChange={onZoomChange ?? (() => {})}
        onClose={onClose}
        onPrev={onPrev}
        onNext={onNext}
        hasPrev={hasPrev}
        hasNext={hasNext}
      />
      <div className="pointer-events-none absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full bg-ink/60 px-3 py-1.5 text-[11px] font-mono text-white">
        Embedded preview — download for full quality
      </div>
    </div>
  )
}
