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

// Moved components (desktop v2 B01). Web keeps its old import paths as re-export stubs.
export * from './file-icon'
export * from './file-browser-row'
export * from './icons'
export * from './file-list/file-row'
export * from './file-list/file-list-header'
export * from './rename-dialog'
export * from './new-folder-dialog'
export * from './duplicate-file-dialog'
export * from './context-menu'
export * from './new-menu'
export * from './move-modal'
export * from './version-history'
export * from './file-details-panel'
export * from './file-list'
export * from './lib/pending-select-timer'
export * from './lib/mod-key'
export * from './upload-progress'
export * from './storage-usage-bar'
export * from './breadcrumb'
export * from './hooks/use-focus-trap'
export * from './lib/folder-colors'
export * from './lib/new-document'
export * from './lib/file-colors'
export * from './lib/format'
export * from './lib/preview'
export * from './lib/display-markers'
export * from './lib/storage-country'
export * from './link'
export * from './folder-viewer-badge'
export * from './empty-states/empty-activity'
export * from './empty-states/empty-drive'
export * from './empty-states/empty-photos'
export * from './empty-states/empty-recent'
export * from './empty-states/empty-search'
export * from './empty-states/empty-shared'
export * from './empty-states/empty-starred'
export * from './empty-states/empty-state'
export * from './empty-states/empty-trash'
export * from './empty-states/forgot-phrase'
export * from './empty-states/link-expired'
export * from './empty-states/offline-state'
export * from './empty-states/upload-error'
export { SORT_KEY_PREFIX, SORT_OPTIONS, typeGroupRank, type SortKey as FileListSortKey } from './file-list-sort'

// Preview renderers, their pure helpers and the host-supplied decoders (desktop v2 B01).
export * from './preview/audio-preview'
export * from './preview/docx-preview'
export * from './preview/file-kinds'
export * from './preview/file-preview-cards'
export * from './preview/heic-preview'
export * from './preview/image-preview'
export * from './preview/info-rail'
export * from './preview/markdown-preview'
export * from './preview/markdown-safe-link'
export * from './preview/pdf-preview'
export * from './preview/pick-renderer'
export * from './preview/preview-chrome'
export * from './preview/raw-preview'
export * from './preview/text-preview'
export * from './preview/tiff-preview'
export * from './preview/unsupported-preview'
export * from './preview/version-scrubber'
export * from './preview/video-preview'
export * from './preview/xlsx-preview'
export * from './preview/zip-listing-preview'
export * from './lib/markdown-link'
export * from './lib/preview-load-policy'
export * from './lib/raw-embedded-jpeg'
export * from './lib/text-editability'
export * from './lib/zip-listing'
export * from './lib/raw-info-rows'
export * from './lib/transfer-progress-copy'
export * from './lib/transfer-progress-types'
export * from './lib/preview-decoders'
export * from './hooks/use-touch-gestures'

// Editor view parts: CodeMirror, theme, conflict and unsaved-changes dialogs (the save pipeline stays with the host).
export * from './editor/bracket-colors'
export * from './editor/codemirror-editor'
export * from './editor/codemirror-theme'
export * from './editor/conflict-dialog'
export * from './editor/editor-langs'
export * from './editor/minimap-extension'
export * from './editor/unsaved-changes-dialog'
export * from './lib/editor-conflict'
export * from './lib/line-diff'
export * from './lib/bracket-depth'

// Office view parts: ribbons, panes, dialogs and command tables (the document save pipeline stays with the host).
export * from './office/calc-formula-bar'
export * from './office/calc-icons'
export * from './office/calc-ribbon'
export * from './office/calc-sheet-tabs'
export * from './office/calc-status-extra'
export * from './office/char-formatting-controls'
export * from './office/command-palette'
export * from './office/floating-selection-toolbar'
export * from './office/impress-filmstrip'
export * from './office/impress-present-overlay'
export * from './office/impress-ribbon'
export * from './office/office-about'
export * from './office/office-conflict-dialog'
export * from './office/office-engine-host'
export * from './office/office-header'
export * from './office/office-icons'
export * from './office/office-loading-skeleton'
export * from './office/office-status-bar'
export * from './office/outline-pane'
export * from './office/ribbon'
export * from './office/templates-gallery'
export * from './office/zoom-control'
export { type OutlineHeading, type DocStats, type OfficeDocKind, type OfficeBridge, type OfficeEngineWindow } from './lib/office/bb-office-bridge'
export { deriveCalcButtonState, CALC_HOME_COMMANDS, CALC_FORMAT_PALETTE_COMMANDS, CALC_STATE_COMMANDS, CALC_PALETTE_ENTRIES, type CalcCommandGroup, type CalcCommandDef, type CalcButtonState, type CalcPaletteEntry } from './lib/office/calc-commands'
export * from './lib/office/calc-selection-stats'
export * from './lib/office/impress-commands'
export * from './lib/office/loader'
export * from './lib/office/office-conflict'
export * from './lib/office/office-file-kind'
export * from './lib/office/office-magic'
export * from './lib/office/office-open-error'
export * from './lib/office/progress-format'
export * from './lib/office/ribbon-commands'
export * from './lib/office/templates'
export * from './lib/office/use-calc-selection-stats'
