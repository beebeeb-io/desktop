// The TIFF preview with browser-support fallback.
import { useEffect, useState } from 'react'
import { previewDecoders } from '../lib/preview-decoders'
import { ImagePreview } from './image-preview'
import { UnsupportedPreview } from './unsupported-preview'

interface TiffPreviewProps {
  blob: Blob
  filename: string
  zoom: number
  rotation: number
  onZoomChange: (zoom: number) => void
  onClose?: () => void
  onPrev?: () => void
  onNext?: () => void
  hasPrev?: boolean
  hasNext?: boolean
}

/**
 * Chromium has no native TIFF codec (task 1565 finding — a real .tiff
 * upload left the plain `<img>` tag firing 'error' forever, honestly
 * downgraded to UnsupportedPreview but never actually RENDERING). This
 * decodes the TIFF off the main thread (`tiff-decode.worker.ts`, `utif2`)
 * into a PNG blob, then hands off to `ImagePreview` — the same
 * decode-then-delegate pattern `HeicPreview` already uses for HEIC/HEIF.
 */
export function TiffPreview({
  blob,
  filename,
  zoom,
  rotation,
  onZoomChange,
  onClose,
  onPrev,
  onNext,
  hasPrev,
  hasNext,
}: TiffPreviewProps) {
  const [pngBlob, setPngBlob] = useState<Blob | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    setPngBlob(null)
    setFailed(false)

    async function decode() {
      try {
        const result = await previewDecoders().decodeTiffToPng(blob)
        if (cancelled) return
        if (result) setPngBlob(result)
        else setFailed(true)
      } catch {
        if (!cancelled) setFailed(true)
      }
    }

    decode()
    return () => {
      cancelled = true
    }
  }, [blob])

  if (pngBlob) {
    return (
      <ImagePreview
        blob={pngBlob}
        filename={filename}
        zoom={zoom}
        rotation={rotation}
        onZoomChange={onZoomChange}
        onClose={onClose}
        onPrev={onPrev}
        onNext={onNext}
        hasPrev={hasPrev}
        hasNext={hasNext}
      />
    )
  }

  if (failed) {
    return <UnsupportedPreview blob={blob} filename={filename} />
  }

  return (
    <div className="flex flex-col items-center gap-3">
      <div className="h-8 w-8 animate-spin rounded-full border-2 border-line border-t-amber" />
      <span className="text-sm text-ink-3">Decoding TIFF...</span>
    </div>
  )
}
