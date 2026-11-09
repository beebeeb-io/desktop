// Shared's row shapes (B05): what the section draws for a share, an invitation, a request and a person. The desktop
// source fills them from the share commands; the board's fixtures (../../source/fixtures/shared.ts) use them too.

export interface Invitation { id: string; name: string; from: string; isFolder: boolean }
export interface WithMeRow { id: string; name: string; isFolder: boolean; owner: string; items?: number; size?: number | null; modifiedAt?: string }
export interface PeopleShare { id: string; name: string; isFolder: boolean; people: string[]; pending?: boolean; items?: number; size?: number | null }
export interface LinkShare { id: string; name: string; expiresAt: string | null; password: boolean; size?: number | null; isFolder?: boolean }
export interface FileRequestRow {
  id: string
  title: string
  files: number
  /** "12 of 50 files" when the request caps the number of files. */
  maxFiles?: number
  into?: string
  expiresAt?: string | null
  closed?: boolean
}
export type Access = 'Can view' | 'Can edit'
/** `markUnreadable`: this PC holds a comparison for the person it cannot read (Rust stops every new share to them). */
export interface Member { id: string; name: string; email: string; you?: boolean; owner?: boolean; access?: Access; invited?: boolean; compared?: boolean; mismatch?: boolean; keyChanged?: boolean; markUnreadable?: boolean }
