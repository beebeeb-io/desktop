import type { RawExifInfo } from './raw-embedded-jpeg'

// Image decoders the host provides to the RAW and TIFF previews. Web runs them in workers; desktop in Rust.
export interface PreviewDecoders {
  /** TIFF to PNG, or null when the blob cannot be decoded. */
  decodeTiffToPng(blob: Blob): Promise<Blob | null>
  /** The embedded JPEG and EXIF of a camera RAW file. */
  extractRawPreview(blob: Blob): Promise<{ previewBlob: Blob | null; exif: RawExifInfo | null }>
}

let decoders: PreviewDecoders | null = null

export function registerPreviewDecoders(d: PreviewDecoders | null): void {
  decoders = d
}

export function previewDecoders(): PreviewDecoders {
  if (!decoders) throw new Error('previewDecoders: no host registered')
  return decoders
}
