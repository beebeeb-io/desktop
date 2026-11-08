// The desktop DriveSource: Tauri commands backed by the Rust engine and core. Keys, the session token and
// every ciphertext stay in Rust; a command returns decrypted view data only (desktop v2 spec, "Data and
// crypto boundary"). Each method names its command; batch B02 added the `drive_*` ones
// (src-tauri/src/drive_v2). A method with no command throws DriveSourceError('unsupported') so the UI
// shows its honest unsupported state, never invented data.

import { invoke, type InvokeArgs } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { DriveSourceError, type DriveSource, type DriveSourceErrorCode, type ItemId } from '@beebeeb/shared/drive-ui'

/** The reason codes the v2 commands return as `Err("code: detail")`; anything else is `failed`. */
const CODES: DriveSourceErrorCode[] = [
  'offline', 'unsupported', 'not-found', 'name-taken', 'quota', 'read-only', 'conflict', 'locked', 'signed-out', 'failed',
]

async function call<T>(name: string, args?: InvokeArgs): Promise<T> {
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

// Binary content frames (drive_read_bytes / drive_save_bytes): [u32 LE header length][header JSON][bytes].
// Content travels as raw IPC bytes, never base64 JSON.
function frame(header: Record<string, string>, bytes: Uint8Array): Uint8Array {
  const head = new TextEncoder().encode(JSON.stringify(header))
  const out = new Uint8Array(4 + head.length + bytes.length)
  new DataView(out.buffer).setUint32(0, head.length, true)
  out.set(head, 4)
  out.set(bytes, 4 + head.length)
  return out
}

function unframe(buf: ArrayBuffer): { header: Record<string, string>; bytes: Uint8Array } {
  if (buf.byteLength < 4) throw new DriveSourceError('failed', 'frame')
  const len = new DataView(buf).getUint32(0, true)
  if (4 + len > buf.byteLength) throw new DriveSourceError('failed', 'frame')
  const header = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 4, len))) as Record<string, string>
  return { header, bytes: new Uint8Array(buf, 4 + len) }
}

/**
 * Files dropped on the window from File Explorer: the window's native drag-drop event gives their paths
 * (a browser File has none), and Rust copies them into the folder for the engine to upload (DW-0481).
 */
export function uploadDroppedPaths(parentId: ItemId | null, paths: string[]): Promise<{ started: number }> {
  return call('drive_upload_dropped', { parentId, paths })
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
    versions: (id) => call('drive_versions', { id }),                                   // wraps existing list_file_versions
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
    restoreFromTrash: (ids) => call('drive_restore_from_trash', { ids }),               // new: a selection (desktop_trash_restore takes one id)
    // The server deletes for good only with a step-up confirmation (X-Confirm-Token from desktop_confirm_action,
    // the password). The contract's deletePermanently(ids) carries none, so it stays unsupported until the
    // contract grows a confirmation (B02 PR note for the lead).
    deletePermanently: todo('desktop_trash_delete_permanently'),
    restoreVersion: (id, versionId) => call('restore_file_version', { fileId: id, versionId }), // existing
    upload: (parentId, input) =>
      input.kind === 'pick'
        ? call('drive_upload_pick', { parentId, mode: input.mode })                     // new: native picker in Rust
        : todo('drive_upload_dropped')(),  // browser File objects have no path; drops go through uploadDroppedPaths
    open: (id) => call('drive_open', { id }),                                           // new: hydrate, decrypt, open in its app
    readText: (id) => call('drive_read_text', { id }),                                  // new
    saveText: (id, text, baseVersionId) => call('drive_save_text', { id, text, baseVersionId }), // new
    readBytes: async (id) => {                                                          // new: binary IPC response
      const { header, bytes } = unframe(await call<ArrayBuffer>('drive_read_bytes', { id }))
      return { bytes, versionId: header.versionId ?? '' }
    },
    saveBytes: async (id, bytes, baseVersionId) => {                                    // new: raw IPC body
      const answer = await call<{ versionId: string }>('drive_save_bytes', frame({ id, baseVersionId }, bytes))
      return { bytes, versionId: answer.versionId }
    },
    setAvailableOffline: (id, on) => call('set_recursive_pin', { itemId: id, pinned: on }), // existing
    revealInFileManager: (id) => call('drive_reveal', { id }),                          // new: id → rel path → existing reveal_and_open_file
  }
}
