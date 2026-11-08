// B05's commands outside DriveSource (spec 3.2), all through desktopApi's command(). Each one sends only ids, emails,
// permission choices, expiry and (one way, into Rust) a password or code. Nothing comes back that carries a key: a
// link answers with its display form, without the part after #; Rust has already put the full link on the clipboard.
import { command, commandUnavailableLabel, type CommandResult } from '../../../desktopApi'
import type { Access, FileRequestRow, Invitation, LinkShare, Member } from './model'

export interface ShareLists { invitations: Invitation[]; links: LinkShare[]; requests: FileRequestRow[]; pending: string[] }
/** A link made or copied by Rust: `display` is the link without its key part, for a toast. */
export interface CopiedLink { display: string }
export type InviteOutcome = { status: 'shared' } | { status: 'invited' } | { status: 'key-changed'; memberId: string }

export const shareList = () => command<ShareLists>('share_list')
export const shareAccept = (id: string) => command<void>('share_accept', { id })
export const shareDecline = (id: string) => command<void>('share_decline', { id })
export const shareLeave = (id: string) => command<void>('share_leave', { id })
export const sharePeople = (itemId: string) => command<Member[]>('share_people', { itemId })
export const shareInvite = (itemId: string, email: string, access: Access, acceptChangedKey = false) =>
  command<InviteOutcome>('share_invite', { itemId, email, canEdit: access === 'Can edit', acceptChangedKey })
/** The server's permission for a member (`read` / `add` / `write` / `admin`); the UI offers view and edit. */
export const shareSetAccess = (itemId: string, memberId: string, access: Access) =>
  command<void>('share_set_access', { itemId, memberId, permission: access === 'Can edit' ? 'write' : 'read' })
/** Stop sharing: one person (`memberId`) or the whole share or link (`memberId` null). */
export const shareRevoke = (itemId: string, memberId: string | null) => command<void>('share_revoke', { itemId, memberId })
/** Link options from the Files share dialog (DW-0407); none means "copy the live link". Several ids make one bundle link. */
export interface LinkChoices { itemIds?: string[]; expiresHours?: number | null; password?: string | null }
export const shareLinkCopy = (itemId: string, choices: LinkChoices = {}) =>
  command<CopiedLink>('share_link_copy', { itemId, itemIds: choices.itemIds ?? null, expiresHours: choices.expiresHours ?? null, password: choices.password ?? null })
/** The Files dialog's expiry choice in hours; `Never` is null. */
export function expiryHours(choice: string): number | null {
  return ({ 'In 1 day': 24, 'In 7 days': 168, 'In 30 days': 720 } as Record<string, number>)[choice] ?? null
}
export const shareLinkUpdate = (itemId: string, change: { expiresDays?: number | null; password?: string | null }) =>
  command<void>('share_link_update', { itemId, expiresDays: change.expiresDays, setExpiry: 'expiresDays' in change, password: change.password ?? null, setPassword: 'password' in change })
export const shareSafetyNumber = (memberId: string) => command<{ groups: string[] }>('share_safety_number', { memberId })
export const shareMarkCompared = (memberId: string, matched: boolean) => command<void>('share_mark_compared', { memberId, matched })
/** DW-0515 over People, Share anyway: keep this share as it is. The person's mark stays, so later shares to them stay checked. */
export const shareAcceptChangedKey = (memberId: string) => command<void>('share_accept_changed_key', { memberId })

export interface FileRequestInput { title: string; note: string; folderName: string; expiresDays: number | null; maxFiles: number | null; maxBytes: number | null }
export const fileRequestCreate = (input: FileRequestInput) => command<CopiedLink>('file_request_create', { input })
export const fileRequestCopy = (id: string) => command<CopiedLink>('file_request_copy', { id })
export const fileRequestUpdate = (id: string, change: { expiresDays?: number | null; close?: boolean }) =>
  command<void>('file_request_update', { id, expiresDays: change.expiresDays ?? null, setExpiry: 'expiresDays' in change, close: !!change.close })
export const fileRequestRemove = (id: string) => command<void>('file_request_remove', { id })

/** Permanent delete with its step-up inside Rust (crypto rule 11): a password or an authenticator code, never a token. */
export const deletePermanently = (ids: string[], secret: { password?: string; code?: string }) =>
  command<void>('drive_delete_permanently', { ids, password: secret.password ?? null, code: secret.code ?? null })

/** The copy for a failed command: never the host's text. */
export function failureText(r: CommandResult<unknown>): string {
  if (r.ok) return ''
  if (r.unsupported) return commandUnavailableLabel('')
  if (/^unsupported/.test(r.reason)) return commandUnavailableLabel('')
  if (/^offline/.test(r.reason)) return 'You’re offline. Try again when you’re connected.'
  if (/^peer_key_changed/.test(r.reason)) return 'This person’s key has changed. Nothing was shared.'
  if (/^(invalid_credentials|wrong_code)/.test(r.reason)) return 'That didn’t work. Check it and try again.'
  return 'Something went wrong. Try again.'
}
