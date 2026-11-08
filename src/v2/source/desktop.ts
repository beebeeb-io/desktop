// The desktop DriveSource: Tauri commands backed by the Rust engine and core. Keys, the session token and
// every ciphertext stay in Rust; a command returns decrypted view data only (desktop v2 spec, "Data and
// crypto boundary"). Each method names its command. `existing` commands are already registered in
// src-tauri; `new` ones are batch B2's to add (spec, command table). Until then a method throws
// DriveSourceError('unsupported') so the UI shows its honest unsupported state, never invented data.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { DriveSourceError, type DriveSource, type DriveSourceErrorCode } from '@beebeeb/shared/drive-ui'

/** The reason codes the v2 commands return as `Err("code: detail")`; anything else is `failed`. */
const CODES: DriveSourceErrorCode[] = [
  'offline', 'unsupported', 'not-found', 'name-taken', 'quota', 'read-only', 'conflict', 'locked', 'signed-out', 'failed',
]

async function call<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args)
  } catch (error) {
    const text = typeof error === 'string' ? error : error instanceof Error ? error.message : ''
    const code = CODES.find((c) => text.startsWith(`${c}:`) || text === c)
    // A command this build does not have, or no Tauri at all (a plain browser), is `unsupported`.
    if (!code && /not found|unknown command|invoke|__TAURI/i.test(text)) throw new DriveSourceError('unsupported', name)
    throw new DriveSourceError(code ?? 'failed', name)
  }
}

const todo = (command: string) => async (): Promise<never> => {
  throw new DriveSourceError('unsupported', command)
}

export function createDesktopSource(): DriveSource {
  return {
    kind: 'desktop',
    capabilities: { localCopies: true, revealInFileManager: true, office: true, nativePickers: true },

    // Reading
    list: (folderId, sort) => call('drive_list', { folderId, sort }),                    // new
    details: (id) => call('drive_details', { id }),                                     // new
    // new: the decrypted thumbnail as a binary IPC response (tauri::ipc::Response), shown through a blob URL;
    // no asset-protocol scope is widened and no file path reaches the web view.
    thumbnail: async (id, size) => {
      const buf = await call<ArrayBuffer | null>('drive_thumbnail', { id, size })
      return buf && buf.byteLength > 0 ? URL.createObjectURL(new Blob([buf])) : null
    },
    versions: (id) => call('drive_versions', { id }),                                   // new: wraps existing list_file_versions
    search: (query) => call('drive_search', { query }),                                 // new: wraps existing desktop_search_files
    recent: (limit) => call('drive_recent', { limit }),                                 // new: wraps existing desktop_recent_files
    shared: () => call('drive_shared'),                                                 // new: list_shared_roots + share API
    photos: (cursor) => call('drive_photos', { cursor }),                               // new
    trash: () => call('drive_trash'),                                                   // new: wraps existing desktop_trash_list
    storage: () => call('drive_storage'),                                               // new: wraps existing desktop_storage_summary
    sync: () => call('drive_sync_summary'),                                             // new: wraps existing sync_status
    subscribe(listener) {
      // new: the engine emits `drive://event` (DriveEvent JSON) after each applied change.
      let stop: (() => void) | null = null
      let cancelled = false
      listen('drive://event', (e) => listener(e.payload as Parameters<typeof listener>[0]))
        .then((un) => { if (cancelled) un(); else stop = un })
        .catch(() => { /* no Tauri: a plain browser gets no live events */ })
      return () => { cancelled = true; stop?.() }
    },

    // Acting
    createFolder: (parentId, name) => call('drive_create_folder', { parentId, name }),  // new
    rename: (id, name) => call('drive_rename', { id, name }),                           // new
    move: (ids, targetId) => call('drive_move', { ids, targetId }),                     // new
    duplicate: (ids) => call('drive_duplicate', { ids }),                               // new
    moveToTrash: (ids) => call('drive_trash_items', { ids }),                           // new
    undoTrash: (undoToken) => call('drive_undo_trash', { undoToken }),                  // new
    restoreFromTrash: (ids) => call('desktop_trash_restore', { ids }),                  // existing (B2 checks its args)
    deletePermanently: (ids) => call('desktop_trash_delete_permanently', { ids }),      // existing (B2 checks its args)
    restoreVersion: (id, versionId) => call('restore_file_version', { fileId: id, versionId }), // existing
    upload: (parentId, input) =>
      input.kind === 'pick'
        ? call('drive_upload_pick', { parentId, mode: input.mode })                     // new: native picker in Rust
        : todo('drive_upload_dropped')(),                                               // new (B4): dropped files by path
    open: (id) => call('drive_open', { id }),                                           // new: hydrate, decrypt, open in its app
    readText: (id) => call('drive_read_text', { id }),                                  // new
    saveText: (id, text, baseVersionId) => call('drive_save_text', { id, text, baseVersionId }), // new
    readBytes: todo('drive_read_bytes'),                                                // new (B2): binary IPC response, never base64 JSON
    saveBytes: todo('drive_save_bytes'),                                                // new (B2)
    setAvailableOffline: (id, on) => call('set_recursive_pin', { itemId: id, pinned: on }), // existing
    revealInFileManager: (id) => call('drive_reveal', { id }),                          // new: id → rel path → existing reveal_and_open_file
  }
}
