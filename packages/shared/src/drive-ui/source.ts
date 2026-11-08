// DriveSource: the one data contract behind the shared Drive UI (desktop v2 spec,
// docs/superpowers/specs/2026-11-08-desktop-v2-design.md in the workspace).
//
// The shared components never call the API, never touch WASM and never see a key. They read
// view models through a DriveSource and ask it to act. Three implementations exist:
//   - web:      today's API client + WASM, keys in the browser as today (repos/web, src/lib/drive-source-web.ts)
//   - desktop:  Tauri commands backed by the Rust engine and core; keys stay in Rust (repos/desktop)
//   - fixtures: static data for headless screenshots and tests (repos/desktop, src/v2/source/fixtures.ts)
//
// Rules every implementation keeps:
//   - Names, sizes and times arrive DECRYPTED for display. Nothing here carries key material, a
//     wrapped key, a session token, a presigned URL or a server ciphertext.
//   - Ids are opaque strings. A component never parses or derives one.
//   - A failure throws DriveSourceError with a code the UI maps to copy; a message is never shown raw.

export type ItemId = string

/** Where the bytes of a file are on this device. `null` on web (no local copy). */
export type LocalState = 'online-only' | 'downloaded' | 'available-offline' | 'syncing' | 'failed'

export type SharedState =
  | { kind: 'none' }
  | { kind: 'link' }
  | { kind: 'people'; people: { name: string; initials: string }[] }

export interface DriveItem {
  id: ItemId
  parentId: ItemId | null
  /** Decrypted display name. */
  name: string
  isFolder: boolean
  /** Bytes; null for a folder. */
  size: number | null
  /** Folders only: the number of direct children, when known. */
  childCount?: number
  /** ISO 8601. */
  modifiedAt: string
  mime?: string
  shared: SharedState
  local: LocalState | null
  starred?: boolean
  readOnly?: boolean
  hasThumbnail: boolean
  /** Two-letter country of the storage pool, never a city (LD-0021). */
  storageCountry?: string
}

export interface Crumb {
  id: ItemId | null
  name: string
}

export interface Listing {
  folder: Crumb
  path: Crumb[]
  items: DriveItem[]
  /** ISO time the listing reflects; offline banners say how old it is (DW-0101). */
  asOf: string
  stale: boolean
}

export interface VersionEntry {
  id: string
  createdAt: string
  size: number
  current: boolean
  deviceName?: string
}

export interface ItemDetails {
  item: DriveItem
  path: Crumb[]
  changedOn?: string
  versionCount: number
  note?: string
}

export interface TrashEntry {
  item: DriveItem
  trashedAt: string
  originalPath: string
}

export interface StorageSummary {
  usedBytes: number
  quotaBytes: number
  filesBytes: number
  photosBytes: number
  backupsBytes: number
}

export interface SyncSummary {
  phase: 'checking' | 'up-to-date' | 'syncing' | 'paused' | 'offline' | 'needs-decision' | 'stopped'
  onDevice: number
  availableOffline: number
  onlineOnly: number
  pending: number
}

export interface ShareEntry {
  item: DriveItem
  direction: 'by-me' | 'with-me'
  people: { name: string; email?: string; role: 'viewer' | 'editor' }[]
  hasLink: boolean
}

export interface PhotoEntry {
  item: DriveItem
  takenAt: string
}

export type SortKey = 'name' | 'modified' | 'size' | 'kind'
export type SortDir = 'asc' | 'desc'

export type DriveEvent =
  | { kind: 'changed'; folderId: ItemId | null }
  | { kind: 'item'; item: DriveItem }
  | { kind: 'removed'; id: ItemId }
  | { kind: 'sync'; sync: SyncSummary }
  | { kind: 'storage'; storage: StorageSummary }

export type DriveSourceErrorCode =
  | 'offline'
  | 'unsupported'
  | 'not-found'
  | 'name-taken'
  | 'quota'
  | 'read-only'
  | 'conflict'
  | 'locked'
  | 'signed-out'
  | 'failed'

export class DriveSourceError extends Error {
  readonly code: DriveSourceErrorCode
  constructor(code: DriveSourceErrorCode, detail?: string) {
    super(detail ?? code)
    this.code = code
    this.name = 'DriveSourceError'
  }
}

export interface DriveCapabilities {
  /** Keep-on-this-device switch, the PC column, This PC section (desktop). */
  localCopies: boolean
  /** "Show in File Explorer" / "Show in Finder". */
  revealInFileManager: boolean
  /** Office editor (Docs, Sheets, Slides). */
  office: boolean
  /** Native pickers (desktop) instead of <input type="file"> (web). */
  nativePickers: boolean
}

/** What an upload starts from: browser File objects on web, a native picker on desktop. */
export type UploadInput = { kind: 'files'; files: File[] } | { kind: 'pick'; mode: 'files' | 'folder' | 'photos' }

export interface TextDocument {
  text: string
  /** The version the text was read from; a save names it so a concurrent edit is a conflict, not an overwrite. */
  versionId: string
}

export interface BinaryDocument {
  /** The decrypted file content (Office documents, viewers). */
  bytes: Uint8Array
  versionId: string
}

export interface DriveSource {
  readonly kind: 'web' | 'desktop' | 'fixtures'
  readonly capabilities: DriveCapabilities

  // Reading
  list(folderId: ItemId | null, sort?: { key: SortKey; dir: SortDir }): Promise<Listing>
  details(id: ItemId): Promise<ItemDetails>
  /** An image URL the web view may load (object URL or asset URL of the DECRYPTED thumbnail), or null. */
  thumbnail(id: ItemId, size: 'small' | 'large'): Promise<string | null>
  versions(id: ItemId): Promise<VersionEntry[]>
  search(query: string): Promise<DriveItem[]>
  recent(limit?: number): Promise<DriveItem[]>
  shared(): Promise<ShareEntry[]>
  photos(cursor?: string): Promise<{ entries: PhotoEntry[]; next?: string }>
  trash(): Promise<TrashEntry[]>
  storage(): Promise<StorageSummary>
  sync(): Promise<SyncSummary>
  subscribe(listener: (event: DriveEvent) => void): () => void

  // Acting
  createFolder(parentId: ItemId | null, name: string): Promise<DriveItem>
  rename(id: ItemId, name: string): Promise<DriveItem>
  move(ids: ItemId[], targetId: ItemId | null): Promise<void>
  duplicate(ids: ItemId[]): Promise<DriveItem[]>
  /** Returns a token Undo hands back (DW-0405, DW-0108). */
  moveToTrash(ids: ItemId[]): Promise<{ undoToken: string }>
  undoTrash(undoToken: string): Promise<void>
  restoreFromTrash(ids: ItemId[]): Promise<void>
  deletePermanently(ids: ItemId[]): Promise<void>
  restoreVersion(id: ItemId, versionId: string): Promise<void>
  upload(parentId: ItemId | null, input: UploadInput): Promise<{ started: number }>
  /** Desktop: decrypt on this device and open in the default app. Web: the in-app preview. */
  open(id: ItemId): Promise<void>
  readText(id: ItemId): Promise<TextDocument>
  saveText(id: ItemId, text: string, baseVersionId: string): Promise<TextDocument>
  readBytes(id: ItemId): Promise<BinaryDocument>
  saveBytes(id: ItemId, bytes: Uint8Array, baseVersionId: string): Promise<BinaryDocument>
  setAvailableOffline(id: ItemId, on: boolean): Promise<void>
  revealInFileManager(id: ItemId): Promise<void>
}
