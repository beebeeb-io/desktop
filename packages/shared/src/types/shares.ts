// Shared share types: options, share views, items, and bundle requests.
export interface ShareOptions {
  expires_in_hours?: number | null
  max_opens?: number | null
  passphrase?: string
  /**
   * When set, the client has wrapped the file key under a client-generated key
   * (double-encrypted mode). The server stores this opaque blob and returns it
   * to recipients — the server cannot decrypt it without the client key that
   * lives only in the URL fragment.
   * Format: base64(nonce(12) || AES-256-GCM-ciphertext(48)) = 80 chars.
   */
  wrapped_file_key?: string
  /**
   * 0709 A+ owner-recoverable share (all-or-nothing with the two wrapped blobs):
   * a client-generated raw token — URL-safe-no-pad base64 of 20 random bytes
   * (27 chars, [A-Za-z0-9_-]). The server validates + hashes it (as today) and
   * uses it instead of generating one. Omit all three for a legacy server-gen
   * token. Sending only some of the three → typed 400.
   */
  token?: string
  /** base64(AES-GCM(masterKey, K_c)) — lets the OWNER re-copy a working link. */
  owner_wrapped_key?: string
  /** base64(AES-GCM(masterKey, tokenUtf8)) — the raw token, wrapped for re-copy. */
  owner_wrapped_token?: string
}

export interface ShareInfo {
  id: string
  token: string
  url: string
  expires_at: string | null
  max_opens: number | null
  has_passphrase: boolean
  created_at: string
  /** True when the client supplied wrapped_file_key at creation time. */
  double_encrypted?: boolean
}

/**
 * Discriminator for the public-link share surface (task 0417).
 * `undefined` on a wire response ⇒ treat as legacy 'file' (the server's
 * `shares.share_type` column is NOT NULL DEFAULT 'file').
 */
export type ShareType = 'file' | 'folder' | 'bundle'

/**
 * One ordered item in a bundle share (task 0417). Returned in the public
 * manifest's `items` array. `name_encrypted` comes from the file's own
 * `files.name_encrypted` (the server never sees plaintext); `wrapped_file_key`
 * is the item's FileKey wrapped under the bundle's fragment key K_c — opaque
 * to the server.
 */
export interface ShareItem {
  file_id: string
  position: number
  name_encrypted: string
  size: number
  chunk_count: number
  /** base64, wrapped under K_c (the URL-fragment key). */
  wrapped_file_key: string
}

export interface ShareView {
  id: string
  name_encrypted?: string
  size_bytes?: number
  chunk_count?: number
  mime_type?: string
  /**
   * Removed by task 0452 — the server intentionally no longer returns any
   * identifier of the share creator to anonymous recipients. Keep this comment
   * so future agents don't try to plumb it back through.
   */
  /** User ID of the person who created the share — for referral attribution. */
  sharer_id?: string
  expires_at?: string | null
  max_opens?: number | null
  open_count?: number
  created_at?: string
  requires_passphrase?: boolean
  error?: string
  message?: string
  /**
   * True when the share was created in double-encrypted mode.
   * When true, wrapped_file_key is present and the #key= fragment contains
   * the client key K_c — not the file key directly.
   * Recipients must: fileKey = AES-GCM-Decrypt(K_c, wrapped_file_key).
   */
  double_encrypted?: boolean
  /**
   * Base64-encoded wrapped file key (nonce || ciphertext, 60 bytes).
   * Only present when double_encrypted === true.
   * The server stores this opaque blob; decryption requires K_c from the URL fragment.
   */
  wrapped_file_key?: string
  /**
   * Bundle shares (task 0417). `undefined` ⇒ legacy single-file share.
   * Clients MUST gate on this before rendering: an unknown/unimplemented
   * `share_type` should show the "Update Beebeeb to open this share" card
   * rather than misrender a single-file view.
   */
  share_type?: ShareType
  /** Bundle only: number of items in the bundle. */
  item_count?: number
  /** Bundle only: the ordered manifest items (present iff share_type === 'bundle'). */
  items?: ShareItem[]
  /** Bundle only: keyset-pagination cursor; null ⇒ no further pages. */
  next_cursor?: number | null
}

/**
 * Request body for creating a bundle share (task 0417) —
 * `POST /api/v1/shares` with `share_type: 'bundle'`. Additive variant of the
 * single-file create path; `file_id`/top-level `wrapped_file_key` are replaced
 * by the ordered `items` array. The envelope-level fields (`expires_in_hours`,
 * `max_opens`, `passphrase`, `token`, owner-recovery blobs) match the
 * single-file share options and apply once to the whole bundle.
 */
export interface CreateBundleShareRequest {
  share_type: 'bundle'
  items: Array<{ file_id: string; position: number; wrapped_file_key: string }>
  expires_in_hours?: number
  max_opens?: number | null
  passphrase?: string | null
  token?: string
  owner_wrapped_key?: string
  owner_wrapped_token?: string
}

export interface MyShare {
  id: string
  file_id: string
  /** NOTE: the RAW token is SHA-256-hashed at rest, so /shares/mine returns it
   *  UNDEFINED in practice (the type still says string for back-compat; guard
   *  before use). The owner rebuilds a working link via owner_wrapped_token. */
  token: string
  /** DEPRECATED — being removed server-side. It carried K_s as #key=, the wrong
   *  key for double-encrypted shares. Never build a recipient link from this;
   *  use buildShareLink(share). */
  url: string
  expires_at: string | null
  max_opens: number | null
  open_count: number
  last_opened_at?: string | null
  has_passphrase: boolean
  revoked?: boolean
  created_at: string
  /** owner-wrapped client key K_c — base64 of AES-GCM(masterKey, K_c),
   *  nonce(12)‖ct+tag. Null for legacy shares created before 0709 A+. */
  owner_wrapped_key?: string | null
  /** owner-wrapped RAW token — base64 of AES-GCM(masterKey, tokenUtf8),
   *  nonce(12)‖ct+tag (variable length). Null for legacy shares. */
  owner_wrapped_token?: string | null
  /** 'file' (default) or 'bundle' — task 1545, finding 4. `file` carries no
   *  `.file` data of its own beyond the representative item below. */
  share_type?: string
  /** Live membership count of a bundle share (0 for a single-file share,
   *  which has no share_items rows). Task 1545, finding 4. */
  item_count?: number
  file: {
    name_encrypted: string
    size_bytes: number
    mime_type: string | null
  }
}

export interface SharedFileDownload {
  blob: Blob
  chunkCount: number | null
  originalSize: number | null
}

export interface ShareInvite {
  id: string
  file_id: string
  sender_id: string
  recipient_email: string
  status: string
  created_at: string
  claimed_at?: string
  approved_at?: string
  file_name_encrypted?: string
  sender_email?: string
  sender_public_key?: string
  recipient_public_key?: string
  can_reshare?: boolean
  expires_at?: string | null
  size_bytes?: number
  is_folder?: boolean
  chunk_count?: number
  mime_type?: string
  encrypted_file_key?: string
  is_folder_share?: boolean
  encrypted_folder_key?: string
  encrypted_owner_folder_key?: string
  /** The grant the server enforces (read | add | write | admin). */
  permission?: string
}

export interface InviteActivity {
  recipient_email: string
  created_at: string
  claimed_at: string | null
  approved_at: string | null
  download_count: number
  last_accessed: string | null
  first_accessed: string | null
}

export interface ShareStats {
  active_links: number
  pending_invites: number
  total_downloads: number
}
