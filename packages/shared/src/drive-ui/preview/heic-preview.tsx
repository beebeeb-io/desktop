// The HEIC image preview with browser-support fallback.
import { useEffect, useState } from 'react'
import { ImagePreview } from './image-preview'
import { UnsupportedPreview } from './unsupported-preview'

type HeicDecodeModule = typeof import('@discourse/heic/decode')

let heicDecodeModulePromise: Promise<HeicDecodeModule> | null = null

async function loadHeicDecodeModule(): Promise<HeicDecodeModule> {
  if (!heicDecodeModulePromise) {
    heicDecodeModulePromise = Promise.all([
      import('@discourse/heic/decode'),
      import('@discourse/heic/codec/dec/heic_dec.wasm?url'),
    ]).then(([module, wasm]) => {
      module.init({ locateFile: () => wasm.default })
      return module
    })
  }
  return heicDecodeModulePromise
}

interface HeicPreviewProps {
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

export function HeicPreview({
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
}: HeicPreviewProps) {
  const [convertedBlob, setConvertedBlob] = useState<Blob | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    setConvertedBlob(null)
    setFailed(false)

    async function convert() {
      try {
        const { default: decode } = await loadHeicDecodeModule()
        const imageData = await decode(await blob.arrayBuffer())
        if (cancelled) return

        const canvas = document.createElement('canvas')
        canvas.width = imageData.width
        canvas.height = imageData.height
        const ctx = canvas.getContext('2d')
        if (!ctx) {
          setFailed(true)
          return
        }
        ctx.putImageData(imageData, 0, 0)
        const jpegBlob = await new Promise<Blob | null>((resolve) => {
          canvas.toBlob(resolve, 'image/jpeg', 0.92)
        })
        if (!cancelled) {
          if (jpegBlob) setConvertedBlob(jpegBlob)
          else setFailed(true)
        }
      } catch {
        if (!cancelled) setFailed(true)
      }
    }

    convert()
    return () => {
      cancelled = true
    }
  }, [blob])

  if (convertedBlob) {
    return (
      <ImagePreview
        blob={convertedBlob}
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
      <span className="text-sm text-ink-3">Converting preview...</span>
    </div>
  )
}
