// Sync history's row (B06): the desktop source builds it from the engine's activity; the board's fixtures
// (../../source/fixtures/photos.ts) use the same shape.

export type HistoryScope = 'this-pc' | 'other'
export interface HistoryRow {
  id: string
  name: string
  /** "Uploaded to Contracts", "Downloaded from Anna’s iPhone": what happened and where. */
  what: string
  device: string
  at: string
  scope: HistoryScope
  /** Set on a row that did not sync: why, in plain words. */
  problem?: string
  /** The Beebeeb item the row shows in the window when opened (a trashed file opens Trash). */
  itemId?: string
  trashed?: boolean
  kind: 'file' | 'folder'
}
