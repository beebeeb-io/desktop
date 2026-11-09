// Photos' per-photo facts beyond DriveSource's PhotoEntry (B06): the desktop source reads them with drive_photo_meta;
// the board's fixtures (../../source/fixtures/photos.ts) use the same shape.

/** What the board shows beside a photo that DriveSource's PhotoEntry does not carry yet (reported as a gap). */
export interface PhotoMeta {
  /** Absent live: the engine doesn't know which device took it. */
  device?: string
  duration?: string
  uploading?: boolean
  camera?: string
  dims?: string
  previewBytes?: number
}
