/**
 * Which preview component draws a decrypted blob (W-P2-4 sweep).
 *
 * Lifted verbatim out of file-preview.tsx: the kind dispatch is pure, so the
 * page component keeps only the download/edit side effects. The TIFF and
 * thumbnail-first comments live with the branches they explain.
 */
import { AUDIO_EXTENSIONS_SET, VIDEO_EXTENSIONS_SET, getExtension } from '../lib/preview-load-policy'
import {
  CODE_MIME_TYPES,
  EXT_LANGUAGE,
  HEIC_IMAGE_EXTS,
  HEIC_IMAGE_MIMES,
  IMAGE_EXTENSIONS_SET,
  PPTX_EXTS,
  PPTX_MIMES,
  RAW_IMAGE_EXTS,
  RAW_IMAGE_MIMES,
  TEXT_EXTENSIONS,
  TEXT_MIME_TYPES,
  TIFF_EXTS,
  TIFF_MIMES,
  ZIP_EXTS,
} from './file-kinds'
import type { RawExifInfo } from '../lib/raw-embedded-jpeg'
import { ImagePreview } from './image-preview'
import { PdfPreview } from './pdf-preview'
import { VideoPreview } from './video-preview'
import { AudioPreview } from './audio-preview'
import { MarkdownPreview } from './markdown-preview'
import { TextPreview } from './text-preview'
import { DocxPreview } from './docx-preview'
import { XlsxPreview } from './xlsx-preview'
import { UnsupportedPreview } from './unsupported-preview'
import { RawPreview } from './raw-preview'
import { HeicPreview } from './heic-preview'
import { TiffPreview } from './tiff-preview'
import { ZipListingPreview } from './zip-listing-preview'

export interface ImageControlProps {
  zoom: number
  rotation: number
  onZoomChange: (zoom: number) => void
  onClose?: () => void
  onPrev?: () => void
  onNext?: () => void
  hasPrev?: boolean
  hasNext?: boolean
  /** RAW only (task 1574) — reports the extracted EXIF summary so the Info
   *  rail can show camera/lens/exposure, mirroring mobile's Info sheet. */
  onRawInfo?: (info: RawExifInfo | null) => void
}

export const MIN_ZOOM = 0.5
export const MAX_ZOOM = 4.0
export const ZOOM_STEP = 0.2

export function pickRenderer(
  mimeType: string | null | undefined,
  blob: Blob,
  filename: string,
  imageControls?: ImageControlProps,
  /** True once `blob` is confirmed to be the full downloaded original —
   *  false while it's still a server-generated thumbnail (task 1574 gate,
   *  Codex review round 2, 2026-09-27; see the TIFF branch below and the
   *  doc comment at this function's TIFF check for why this matters). */
  blobIsOriginal = true,
): React.ReactNode {
  const ext = getExtension(filename)

  // Office formats — client-side via lazy-loaded mammoth / SheetJS
  if (
    mimeType === 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' ||
    ext === 'docx'
  ) {
    return <DocxPreview blob={blob} filename={filename} />
  }
  if (
    mimeType === 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' ||
    ext === 'xlsx'
  ) {
    return <XlsxPreview blob={blob} filename={filename} />
  }
  // Presentation formats + legacy .doc/.xls — show download card (see the
  // PPTX_EXTS/PPTX_MIMES doc comment above for why, task 1574).
  if (PPTX_MIMES.has(mimeType ?? '') || PPTX_EXTS.has(ext) || ext === 'doc' || ext === 'xls') {
    return <UnsupportedPreview blob={blob} filename={filename} />
  }

  // ZIP — a listing, not extraction (task 1574).
  if (ZIP_EXTS.has(ext)) {
    return <ZipListingPreview blob={blob} filename={filename} />
  }

  // Camera RAW — extract embedded JPEG preview, fall back to download card
  if (RAW_IMAGE_MIMES.has(mimeType ?? '') || RAW_IMAGE_EXTS.has(ext)) {
    return (
      <RawPreview
        blob={blob}
        filename={filename}
        zoom={imageControls?.zoom ?? 1}
        rotation={imageControls?.rotation ?? 0}
        onZoomChange={imageControls?.onZoomChange ?? (() => {})}
        onClose={imageControls?.onClose}
        onPrev={imageControls?.onPrev}
        onNext={imageControls?.onNext}
        hasPrev={imageControls?.hasPrev}
        hasNext={imageControls?.hasNext}
        onInfo={imageControls?.onRawInfo}
      />
    )
  }

  // HEIC/HEIF — convert the decrypted blob to JPEG client-side for preview.
  if (HEIC_IMAGE_MIMES.has(mimeType ?? '') || HEIC_IMAGE_EXTS.has(ext)) {
    return (
      <HeicPreview
        blob={blob}
        filename={filename}
        zoom={imageControls?.zoom ?? 1}
        rotation={imageControls?.rotation ?? 0}
        onZoomChange={imageControls?.onZoomChange ?? (() => {})}
        onClose={imageControls?.onClose}
        onPrev={imageControls?.onPrev}
        onNext={imageControls?.onNext}
        hasPrev={imageControls?.hasPrev}
        hasNext={imageControls?.hasNext}
      />
    )
  }

  // TIFF — decoded off the main thread via utif2 (task 1574), see
  // TIFF_MIMES/TIFF_EXTS doc comment above. Checked before the generic
  // image branch below, which otherwise fed TIFF bytes straight to a plain
  // `<img>` that Chromium can never decode (task 1565 finding).
  //
  // Gated on `blobIsOriginal` (Codex review round 2, task 1574 gate,
  // 2026-09-27): a current-version TIFF CAN already have a server-side WebP
  // thumbnail (CLI/desktop/mobile decode TIFF via native/rust image
  // libraries the browser doesn't have and generate one on upload, same as
  // any other image — see repos/cli/src/thumbnail.rs). While `blob` is
  // still that thumbnail (`blobIsOriginal` false — the fast thumbnail-first
  // path below hasn't been superseded by the full download yet), routing
  // here would feed WebP bytes to utif2, which rejects them outright. Round
  // 1 of this review "fixed" that by excluding TIFF from thumbnail-first
  // entirely — which traded one bug for a worse one Codex caught in round
  // 2: a TIFF WITH a thumbnail now had to fully download+decrypt the
  // ORIGINAL (however large) before ever getting a chance to render
  // anything, even though a perfectly good thumbnail was sitting right
  // there. This is the actual fix: thumbnail-first stays ENABLED for TIFF
  // (see the `isImageFile` check above), and a still-thumbnail blob falls
  // through to the generic image branch below instead (a plain `<img>`
  // decodes WebP natively, exactly like every other image type) —
  // TiffPreview's real utif2 decode only ever runs once `blobIsOriginal` is
  // true, i.e. against the confirmed real original.
  if (blobIsOriginal && (TIFF_MIMES.has(mimeType ?? '') || TIFF_EXTS.has(ext))) {
    return (
      <TiffPreview
        blob={blob}
        filename={filename}
        zoom={imageControls?.zoom ?? 1}
        rotation={imageControls?.rotation ?? 0}
        onZoomChange={imageControls?.onZoomChange ?? (() => {})}
        onClose={imageControls?.onClose}
        onPrev={imageControls?.onPrev}
        onNext={imageControls?.onNext}
        hasPrev={imageControls?.hasPrev}
        hasNext={imageControls?.hasNext}
      />
    )
  }

  if (mimeType?.startsWith('image/') || IMAGE_EXTENSIONS_SET.has(ext)) {
    return (
      <ImagePreview
        blob={blob}
        filename={filename}
        zoom={imageControls?.zoom ?? 1}
        rotation={imageControls?.rotation ?? 0}
        onZoomChange={imageControls?.onZoomChange ?? (() => {})}
        onClose={imageControls?.onClose}
        onPrev={imageControls?.onPrev}
        onNext={imageControls?.onNext}
        hasPrev={imageControls?.hasPrev}
        hasNext={imageControls?.hasNext}
      />
    )
  }
  if (mimeType === 'application/pdf' || ext === 'pdf') {
    return <PdfPreview blob={blob} />
  }
  if (mimeType?.startsWith('video/') || VIDEO_EXTENSIONS_SET.has(ext)) {
    return <VideoPreview blob={blob} />
  }
  if (mimeType?.startsWith('audio/') || AUDIO_EXTENSIONS_SET.has(ext)) {
    return <AudioPreview blob={blob} filename={filename} />
  }
  if (mimeType === 'text/markdown' || ext === 'md' || ext === 'mdx') {
    return <MarkdownPreview blob={blob} />
  }

  // Code files — text preview with syntax coloring
  const lang = (mimeType ? CODE_MIME_TYPES[mimeType] : undefined) ?? EXT_LANGUAGE[ext]
  if (lang) {
    return <TextPreview blob={blob} language={lang} filename={filename} />
  }

  // Plain text files
  if ((mimeType ? TEXT_MIME_TYPES.has(mimeType) : false) || TEXT_EXTENSIONS.has(ext)) {
    return <TextPreview blob={blob} filename={filename} />
  }

  return null
}
