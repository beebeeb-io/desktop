/**
 * LD-0001 §4 row 5 — the whole-file media cap.
 *
 * A video or audio file over 2 GiB must not be downloaded into a Blob: the
 * preview shows the honest download card instead. Pure so it is testable
 * without a renderer.
 *
 * The video/audio extension sets and the filename→extension rule live HERE and
 * nowhere else: `file-preview.tsx`'s renderer routing and this cap must agree,
 * and a second copy is how they drift apart (round-2 review, item 11).
 */
export const WHOLE_FILE_MEDIA_CAP_BYTES = 2 * 1024 ** 3

export const VIDEO_EXTENSIONS_SET = new Set([
  'mp4', 'mov', 'avi', 'mkv', 'webm', 'm4v', 'ogv', 'hevc',
])
export const AUDIO_EXTENSIONS_SET = new Set([
  'mp3', 'flac', 'wav', 'ogg', 'oga', 'opus', 'm4a', 'aac', 'aiff', 'aif', 'wma', 'alac',
])

/**
 * Lowercased extension, or the lowercased bare filename when it has no dot
 * (`Dockerfile`, `Makefile`, ...). The bare-filename fallback is load-bearing
 * for the preview renderer (task 1565: a real `Dockerfile` fell through every
 * branch and landed on "Preview not available").
 */
export function getExtension(filename: string): string {
  const dot = filename.lastIndexOf('.')
  if (dot === -1) return filename.toLowerCase()
  return filename.slice(dot + 1).toLowerCase()
}

export type MediaKind = 'video' | 'audio'

/**
 * Route exactly as `pickRenderer` does: images and PDFs are rendered before
 * media, then video by MIME OR extension, then audio by MIME OR extension.
 * (`audio/mp4` named `clip.mp4` is a video; `application/pdf` named `x.mp4` is
 * a PDF, never capped.)
 */
export function mediaKind(
  mimeType: string | null | undefined,
  filename: string,
): MediaKind | null {
  const ext = getExtension(filename)
  if (mimeType?.startsWith('image/')) return null
  if (mimeType === 'application/pdf' || ext === 'pdf') return null
  if (mimeType?.startsWith('video/') || VIDEO_EXTENSIONS_SET.has(ext)) return 'video'
  if (mimeType?.startsWith('audio/') || AUDIO_EXTENSIONS_SET.has(ext)) return 'audio'
  return null
}

/** The card's copy for an over-cap video/audio file, or null for anything else. */
export function mediaCapCopy(kind: MediaKind): string {
  if (kind === 'video') return 'This video is too large to play in the browser. Download it to watch.'
  return 'This audio file is too large to play in the browser. Download it to listen.'
}

/** True when this whole-file media load must show the download card instead. */
export function isWholeFileMediaOverCap(
  mimeType: string | null | undefined,
  filename: string,
  sizeBytes: number,
): boolean {
  if (!(sizeBytes > WHOLE_FILE_MEDIA_CAP_BYTES)) return false
  return mediaKind(mimeType, filename) !== null
}

export function wholeFileMediaCapCopy(
  mimeType: string | null | undefined,
  filename: string,
): string | null {
  const kind = mediaKind(mimeType, filename)
  return kind ? mediaCapCopy(kind) : null
}

/**
 * Thrown by the load layer (after the file's AUTHENTICATED metadata is read,
 * before the body is fetched) when a media file is over the cap. The preview
 * catches it and renders the card; the message is the card's copy.
 */
export class TooLargeToPreviewError extends Error {
  readonly kind: MediaKind
  constructor(kind: MediaKind) {
    super(mediaCapCopy(kind))
    this.name = 'TooLargeToPreviewError'
    this.kind = kind
  }
}

/**
 * Which save path a Download action takes:
 * - the already-loaded original Blob → `blob`
 * - the over-cap card, which has no Blob → `stream` (SW streaming save)
 * - anything else (thumbnail still showing, historical version) → `blob`
 *   via `decryptToBlob` / `decryptVersionToBlob`
 */
export type DownloadRoute = 'blob' | 'stream'

export function downloadRoute(hasOriginalBlob: boolean, overCap: boolean): DownloadRoute {
  if (hasOriginalBlob) return 'blob'
  return overCap ? 'stream' : 'blob'
}

/** The parts of a version row the Download decision needs. */
export interface DownloadVersionFacts {
  id: string
  chunk_count: number
  size_bytes: number
}

/** What the preview's Download button does (file-preview.tsx only executes it). */
export type DownloadAction =
  | { kind: 'save-blob' }
  | { kind: 'stream-to-disk'; versionId: string | null; chunkCount: number; sizeBytes: number }
  | { kind: 'decrypt-live' }
  | { kind: 'decrypt-version'; versionId: string; chunkCount: number; sizeBytes: number }
  | { kind: 'version-not-found' }

/**
 * The Download decision, pure, so the branch the preview really takes is the
 * one under test.
 */
export function chooseDownloadAction(facts: {
  hasOriginalBlob: boolean
  overCap: boolean
  selectedVersionId: string | null
  file: { chunk_count: number; size_bytes: number }
  version: DownloadVersionFacts | null
}): DownloadAction {
  const route = downloadRoute(facts.hasOriginalBlob, facts.overCap)
  if (route === 'blob' && facts.hasOriginalBlob) return { kind: 'save-blob' }
  if (route === 'stream') {
    // C1: the over-cap card NEVER buffers the whole file, for the live file
    // AND for a past version: both stream to disk.
    if (facts.selectedVersionId === null) {
      return {
        kind: 'stream-to-disk',
        versionId: null,
        chunkCount: facts.file.chunk_count,
        sizeBytes: facts.file.size_bytes,
      }
    }
    if (!facts.version) return { kind: 'version-not-found' }
    return {
      kind: 'stream-to-disk',
      versionId: facts.version.id,
      chunkCount: facts.version.chunk_count,
      sizeBytes: facts.version.size_bytes,
    }
  }
  if (facts.selectedVersionId === null) return { kind: 'decrypt-live' }
  if (!facts.version) return { kind: 'version-not-found' }
  return {
    kind: 'decrypt-version',
    versionId: facts.version.id,
    chunkCount: facts.version.chunk_count,
    sizeBytes: facts.version.size_bytes,
  }
}
