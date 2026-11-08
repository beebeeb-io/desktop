// Batch B02b: the data the screens draw beyond DriveSource's first contract (PUNCHLIST "GAP (data)" items from B03,
// B04, B05, B06, B07, B09). One interface, two sides: the desktop side calls the `drive_*` commands in
// src-tauri/src/drive_v2/status_cmds.rs; a plain browser gets the board's fixtures (./fixtures/extras.ts).
//
// Folding these into `DriveSource` itself goes through B01's contract (packages/shared is a generated mirror and
// frozen here); until then this module is that one contract change, desktop-side. Data only: never a key, a token,
// a URL or an absolute path. A field the engine can't know is absent, and the UI shows the board's honest empty state.

import { useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { DriveItem, ItemId } from '@beebeeb/shared/drive-ui'
import { hasTauri } from './index'
import { createFixtureExtras } from './fixtures/extras'

export interface ListingProgress { done: number; total: number }
export interface ConflictRef { id: ItemId; name: string }
export type BackupState = 'off' | 'getting-ready' | 'on' | 'stopped'
export interface BackupView {
  state: BackupState
  /** ISO time of the newest finished pass; absent before the first. */
  lastBackupAt?: string
  /** This PC's folder in Backups; absent before the layout is known. */
  deviceFolder?: string
  foldersOn: number
  foldersGettingReady: number
}

/** The engine snapshot fields the flyout, the banners, Backups and the getting-ready strip draw. */
export interface DriveStatus {
  signedOut: boolean
  /** Absent: this build doesn't read the connection's cost. */
  metered?: boolean
  stoppedFiles: number
  syncProblems: number
  backupProblems: number
  uploadBps?: number
  etaSeconds?: number
  fileCount?: number
  freeSpaceBytes?: number
  backup: BackupView
  listing?: ListingProgress
  conflicts: ConflictRef[]
}

export interface TransferRow {
  /** The file id while moving; the queue's change id while waiting or stopped (what `retry` takes). */
  id: string
  fileId?: ItemId
  name: string
  folder: string
  direction: 'up' | 'down'
  state: 'moving' | 'waiting' | 'stopped'
  bytes: number
  total?: number
  reason?: string
  retryable: boolean
}

export interface SyncProblem {
  id: string
  fileId?: ItemId
  name: string
  folder: string
  reason: string
  retryable: boolean
  backup: boolean
  at?: string
}

export interface OfflineEntry { item: DriveItem; items?: number; bytes: number }

export interface PhotoMeta {
  takenAt?: string
  camera?: string
  width?: number
  height?: number
  durationSeconds?: number
  uploading: boolean
}

export interface PhotoSource { id: ItemId | null; name: string; path: string; count: number }

export interface DriveExtras {
  status(): Promise<DriveStatus>
  transfers(): Promise<TransferRow[]>
  /** The moving rows, about once a second while anything moves. */
  subscribeTransfers(listener: (rows: TransferRow[]) => void): () => void
  syncProblems(): Promise<SyncProblem[]>
  retry(ids: string[]): Promise<{ retried: number }>
  offlineList(): Promise<OfflineEntry[]>
  photoMeta(ids: ItemId[]): Promise<Record<ItemId, PhotoMeta>>
  photoSources(): Promise<PhotoSource[]>
  /** "Files › Docs" for each id, for search results. */
  paths(ids: ItemId[]): Promise<Record<ItemId, string>>
}

export function createDesktopExtras(): DriveExtras {
  return {
    status: () => invoke('drive_status'),
    transfers: () => invoke('drive_transfers'),
    subscribeTransfers(listener) {
      let stop: (() => void) | null = null
      let cancelled = false
      listen<TransferRow[]>('drive://transfers', (e) => listener(e.payload))
        .then((un) => { if (cancelled) un(); else stop = un })
        .catch(() => { /* no Tauri: no live progress */ })
      return () => { cancelled = true; stop?.() }
    },
    syncProblems: () => invoke('drive_sync_problems'),
    retry: (ids) => invoke('drive_sync_retry', { ids }),
    offlineList: () => invoke('drive_offline_list'),
    photoMeta: (ids) => (ids.length ? invoke('drive_photo_meta', { ids }) : Promise.resolve({})),
    photoSources: () => invoke('drive_photo_sources'),
    paths: (ids) => (ids.length ? invoke('drive_paths', { ids }) : Promise.resolve({})),
  }
}

/** Same rule as `pickSource`: `?source=fixtures` or a plain browser gets the fixtures. */
export function pickExtras(search: string = typeof window === 'undefined' ? '' : window.location.search): DriveExtras {
  const params = new URLSearchParams(search)
  if (params.get('source') === 'fixtures' || !hasTauri()) return createFixtureExtras(params.get('scenario') ?? 'default')
  return createDesktopExtras()
}

let shared: DriveExtras | null = null
/** One extras source per window. */
export function useDriveExtras(): DriveExtras {
  return useMemo(() => (shared ??= pickExtras()), [])
}
