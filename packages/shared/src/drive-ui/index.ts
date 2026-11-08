// @beebeeb/shared/drive-ui: the Drive UI shared by web and desktop, and the DriveSource contract behind it.
// Import rule: nothing under drive-ui imports ../api, ../telemetry, beebeeb-wasm or any app code; only react and ../design.
// Components arrive here batch by batch (desktop v2 BATCHES.md, batch B1); each keeps its web behaviour.
export { DriveSourceError } from './source'
export type {
  BinaryDocument,
  Crumb,
  DriveCapabilities,
  DriveEvent,
  DriveItem,
  DriveSource,
  DriveSourceErrorCode,
  ItemDetails,
  ItemId,
  Listing,
  LocalState,
  PhotoEntry,
  ShareEntry,
  SharedState,
  SortDir,
  SortKey,
  StorageSummary,
  SyncSummary,
  TextDocument,
  TrashEntry,
  UploadInput,
  VersionEntry,
} from './source'
export { DriveSourceProvider, useDriveSource } from './provider'
