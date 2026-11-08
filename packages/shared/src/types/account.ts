// Shared account types: passkeys, activity, and recovery state.
export interface PasskeyInfo {
  id: string
  name: string
  created_at: string
}

export interface PasskeyRegisterStartResponse {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  publicKey: any
  /**
   * Task 1493: the registration challenge state now stays server-side
   * (single-use, 5-minute TTL, bound to the caller's user_id). The client
   * only gets this opaque id back and echoes it to `register-finish` — it
   * never sees or edits the actual WebAuthn state any more (was
   * `reg_state: string`, a client-editable base64 blob).
   */
  reg_id: string
}

export interface PasskeyLoginStartResponse {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  publicKey: any
  auth_state: string
  user_id: string
}

export interface ActivityEvent {
  id: string
  type: string
  subject: string | null
  details: string | null
  where: string | null
  /** Client that triggered the event — "web" | "mobile-ios" | "mobile-android" | "desktop" | "cli" */
  device?: string | null
  created_at: string
}

export interface ActivityResponse {
  events: ActivityEvent[]
  total: number
  page: number
}

export interface ActivityEncryptedNameSnapshot {
  file_id: string
  name_encrypted?: string | null
  old_name_encrypted?: string | null
  new_name_encrypted?: string | null
}

export interface AccountActivityEvent {
  id: string
  type: string
  description: string
  category: string
  outcome: string
  device: string | null
  country_code: string | null
  encrypted_name_snapshot?: ActivityEncryptedNameSnapshot | null
  created_at: string
}

export interface AccountActivitySummary {
  last_login_at: string | null
  last_login_device: string | null
  active_sessions: number
  active_shares: number
  security_score: string
}

export interface AccountActivity {
  events: AccountActivityEvent[]
  summary: AccountActivitySummary
}

export interface SecurityFactor {
  key: string
  satisfied: boolean
}

export interface SecurityScore {
  score: number
  max: number
  label: string
  factors: SecurityFactor[]
}

export interface AccountSession {
  id: string
  device_name: string
  device_kind: string
  country_code: string | null
  last_active_at: string | null
  created_at: string
  is_current: boolean
}

export interface Workspace {
  id: string
  name: string
  owner_id: string
  role: string
  created_at: string
}

export interface WorkspaceMemberDetail {
  id: string
  email: string
  role: string
  joined_at: string
}

export interface PendingInvite {
  id: string
  email: string
  role: string
  invited_by: string
  created_at: string
  expires_at: string
}

export interface WorkspaceMembersResponse {
  members: WorkspaceMemberDetail[]
  pending_invites: PendingInvite[]
}

export interface InviteInfo {
  id: string
  email: string
  role: string
  token: string
  workspace_name: string
  invited_by: string
  expires_at: string
  created_at: string
}

export interface AcceptInviteResponse {
  message: string
  workspace_id: string
  workspace_name: string
  role?: string
  invited_by?: string
}

export interface InvitePreview {
  workspace_name: string
  invited_by: string
  role: string
  expires_at: string
}
