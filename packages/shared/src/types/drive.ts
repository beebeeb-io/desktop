// Shared drive types: files, uploads, sync nodes, ops, and snapshots.
import type { components } from './openapi'
import type { DownloadVersion } from './auth'

export interface DriveFile {
  id: string
  name_encrypted: string
  /** Null for files uploaded after the ZK audit fix (commit 04d5288).
   *  Infer type from decrypted filename extension instead. */
  mime_type: string | null
  size_bytes: number
  is_folder: boolean
  /** True when the file has been moved to trash. Returned by list_files. */
  is_trashed: boolean
  parent_id: string | null
  chunk_count: number
  /** The pinned object version (LD-0001). Absent on older responses. */
  download_version?: DownloadVersion | null
  is_starred?: boolean
  /** True when the file is an image or video. Set by the client at upload time
   *  because MIME types are encrypted — the server cannot infer media type.
   *  Returned by the server on all file-list responses. */
  is_media?: boolean
  has_thumbnail?: boolean
  /** True when the file has a large (1600px) thumbnail variant in addition to the
   *  medium one. Returned by the server on all file responses. Optional because
   *  mobile/CLI/legacy uploads only have a medium variant — the preview gates its
   *  `/thumbnail/large` request on this flag to avoid a guaranteed 404. */
  has_large_thumbnail?: boolean
  version_number?: number
  /** Number of active (non-revoked, non-expired) share links for this file.
   *  Populated by a LEFT JOIN in the server's list_files handler. */
  share_count?: number
  /** Encrypted file note. JSON string { nonce: string, ciphertext: string } (base64).
   *  Null when no note has been set. Decrypted client-side with the file key. */
  note_encrypted?: string | null
  /** Set only on files received through a file request (the inverse-share flow).
   *  The owner decrypts these via the request-key path (`openRequestUpload`)
   *  instead of the normal `derive_file_key(master, file_id)` path:
   *    content_key C = open_request_upload(R_priv, sender_ephemeral_pubkey,
   *                                        EMPTY_FILE_REQUEST_ID, wrapped_content_key)
   *  where R_priv is unwrapped from the owning request's wrapped_private_key.
   *  All three are base64 (opaque server-side). Absent for normal vault files.
   *  NOTE: the server must include these on file-list/get responses for the
   *  owner-decrypt branch to activate — see file-request-crypto.ts. */
  file_request_id?: string | null
  sender_ephemeral_pubkey?: string | null
  wrapped_content_key?: string | null
  /** Task 2237b: a guest- or member-added row in a shared folder. The owner's
   *  own copy of the file key, wrapped to the master key; null until an owner
   *  client adopts the file. */
  owner_wrapped_file_key?: string | null
  /** The folder link the guest used, for "added via {label}" attribution. */
  acted_via_share_id?: string | null
  /** The guest's chosen display name, sealed under the folder key. */
  actor_name_encrypted?: string | null
  /** The signed-in member who added the file, when it was not a guest. */
  uploaded_by?: string | null
  created_at: string
  updated_at: string
}

export type UploadStatusResponse = components['schemas']['UploadStatusResponse']

/** Response from `POST /api/v1/files/:id/upload/abandon` (task 1571). */
export interface UploadAbandonResponse {
  id: string
  status: 'reverted' | 'deleted' | 'not_uploading'
  previous_version_number?: number
  deleted: boolean
}

export interface SyncNode {
  id: string
  name_encrypted: string
  file_request_id?: string | null
  sender_ephemeral_pubkey?: string | null
  wrapped_content_key?: string | null
  parent_id: string | null
  is_folder: boolean
  size_bytes: number
  mime_type: string | null
  content_hash: string | null
  version_number: number
  has_thumbnail: boolean
  has_large_thumbnail: boolean
  storage_pool_id: string | null
  is_trashed: boolean
  is_starred: boolean
  chunk_count?: number
  created_at: string
  updated_at: string
}

export interface SyncOp {
  seq_id: number
  op_type: string
  client_op_id?: string
  payload: Record<string, unknown>
  device_id?: string
  created_at: string
}

export interface SyncSnapshot {
  seq_id: number
  nodes: SyncNode[]
}

export interface SubmittedSyncOp {
  client_op_id: string
  op_type: string
  payload: Record<string, unknown>
  device_id?: string
}

export interface SyncOpsResult {
  applied: { seq_id: number; client_op_id: string }[]
  rejected: {
    client_op_id: string
    reason: string
    winning_op?: { op_type: string; payload: Record<string, unknown> }
  }[]
}

export interface StreamTokenResponse {
  stream_token: string
  expires_at: string
}

export interface AccountExport {
  user_id: string
  email: string
  exported_at: string
  preferences: Record<string, unknown>
  files: unknown[]
  shares: unknown[]
}
