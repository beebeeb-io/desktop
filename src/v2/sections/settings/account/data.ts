// B14 data layer: every read and write goes through command() (never the shared API, never a key or token).
// A plain browser (headless frames, `bun run dev`) has no Rust behind the commands: the panes start from the app's own
// empty state and show the host's honest refusal, never example rows.
import { useCallback, useEffect, useState } from 'react'
import { appVersionFields } from '../../../../appVersion'
import { command, commandUnavailableLabel, type AccountProfile, type AccountActivity, type AccountSessionList, type CommandResult, type DesktopPlatform, type SecurityScore, type SyncStatus } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { BREACHED, BREACH_BLOCKED } from '../../../auth/create/model'
import type { ActivityEvent, DeviceGroup, Factor, Passkey, ProductRow } from './types'

export type Scenario = 'default' | 'no-passkey' | 'no-passkeys-at-all' | 'logging-off'

export interface AccountState {
  name: string
  email: string
  emailVerified: boolean
  passwordChangedAt: string | null
  totp: { on: boolean; addedAt: string | null; lastUsedAt: string | null; codesLeft: number | null; codesTotal: number }
  activityLogging: boolean
  products: ProductRow[]
  devices: DeviceGroup[]
  passkeys: Passkey[]
  factors: Factor[]
  activity: ActivityEvent[]
}

/** The app's own empty defaults: what a pane shows before any command answers. */
export function initialAccountState(): AccountState {
  return {
    name: '',
    email: '',
    emailVerified: false,
    passwordChangedAt: null,
    totp: { on: false, addedAt: null, lastUsedAt: null, codesLeft: null, codesTotal: 8 },
    activityLogging: true,
    products: [{ name: 'Drive', state: 'This app' }],
    devices: [],
    passkeys: [],
    factors: [],
    activity: [],
  }
}

/**
 * How long the account row and the Account page wait for `account_profile` before saying the network is unreachable.
 * A read that answers clears the timer; a read that never answers must not leave the page blank or the row on "Account".
 */
export const PROFILE_TIMEOUT_MS = 8000

/**
 * The sidebar's account row (DW-0401). While the read is in flight it says "Account"; once it answers it shows the
 * display name, else the email. A failed read with nothing known is `''`: the row is empty and the Account page holds
 * the error, so "Account" never stays on screen as if it were the person's name.
 */
export function accountRowLabel(display: string | null | undefined, email: string | null | undefined, pending: boolean): string {
  if (pending) return 'Account'
  const name = display?.trim() ?? ''
  if (name) return name
  return email?.trim() ?? ''
}

const OS: Record<string, string> = { app: 'Desktop app', windows: 'Windows', macos: 'macOS', linux: 'Linux', ios: 'iOS', android: 'Android', web: 'Web', cli: 'Command line' }

/** Server sessions grouped by device name, so the products read as one list (DW-A55). */
function groupSessions(list: AccountSessionList): DeviceGroup[] {
  const by = new Map<string, DeviceGroup>()
  for (const s of list.sessions) {
    const key = `${s.device_name}\u0000${s.device_kind}`
    const g = by.get(key)
    if (g) {
      g.session_ids.push(s.id)
      g.is_current ||= s.is_current
      if ((s.last_active_at ?? '') > (g.last_active_at ?? '')) g.last_active_at = s.last_active_at ?? null
    } else {
      by.set(key, { session_ids: [s.id], name: s.device_name, os: OS[s.device_kind] ?? s.device_kind, apps: ['Drive'], last_active_at: s.last_active_at ?? null, country_code: s.country_code ?? null, is_current: s.is_current })
    }
  }
  return [...by.values()].sort((a, b) => Number(b.is_current) - Number(a.is_current) || (b.last_active_at ?? '').localeCompare(a.last_active_at ?? ''))
}

/**
 * Reads every account fact once and again on `reload()` (the checkup checks again each time it opens). A `Scenario`
 * only names a board frame for the frames' answer table (M4); it never injects example rows into the running app.
 */
export function useAccount(scenario: Scenario = 'default') {
  void scenario
  const [state, setState] = useState<AccountState>(initialAccountState)
  const [error, setError] = useState<string | null>(null)
  // Kept off AccountState: an empty `devices` array is also the state before any read, and a failed list must not
  // become that. Null until `account_session_list` answers.
  const [sessionList, setSessionList] = useState<CommandResult<AccountSessionList> | null>(null)
  const [localPc, setLocalPc] = useState<Pick<DevicesPaneInput, 'sync' | 'version' | 'platform'>>({ sync: null, version: null, platform: null })
  const reload = useCallback(async () => {
    // A profile read that never answers must not leave the page blank: after PROFILE_TIMEOUT_MS it says the network is
    // unreachable (the existing sentence) and offers Try again. A read that answers clears the timer, so a slow but
    // working server is never called unreachable.
    let profileAnswered = false
    // The literal is PROFILE_TIMEOUT_MS: the test harness runs this hook with only its own bindings, so a module
    // constant here would be a free name it cannot resolve.
    const timer = window.setTimeout(() => {
      if (!profileAnswered) setError('network')
    }, 8000)
    const profileRead = command<AccountProfile>('account_profile').then((r) => {
      profileAnswered = true
      window.clearTimeout(timer)
      return r
    })
    const [profile, name, sessions, score, activity, logging, passkeys, sync, version, platform] = await Promise.all([
      profileRead,
      command<{ display_name: string | null }>('account_display_name'),
      command<AccountSessionList>('account_session_list'),
      command<SecurityScore>('account_security_score'),
      command<AccountActivity>('account_activity'),
      command<{ on: boolean }>('account_activity_logging'),
      command<Passkey[]>('account_passkeys_list'),
      command<Pick<SyncStatus, 'logged_in'>>('sync_status'),
      command<unknown>('app_version'),
      command<DesktopPlatform>('desktop_platform'),
    ])
    const firstError = [profile, sessions, score].find((r) => !r.ok)
    setError(firstError && !firstError.ok ? firstError.reason : null)
    setSessionList(sessions)
    setLocalPc({ sync, version, platform })
    setState((s) => ({
      ...s,
      name: name.ok ? name.value.display_name ?? '' : s.name,
      email: profile.ok ? profile.value.email : s.email,
      emailVerified: profile.ok ? profile.value.email_verified : s.emailVerified,
      passwordChangedAt: null,
      totp: profile.ok ? { on: !!profile.value.totp_enabled, addedAt: null, lastUsedAt: null, codesLeft: null, codesTotal: 8 } : s.totp,
      activityLogging: logging.ok ? logging.value.on : s.activityLogging,
      products: [{ name: 'Drive', state: 'This app' }, ...s.products.filter((p) => p.name !== 'Drive').map((p) => ({ ...p, state: 'Not signed in' as const }))],
      devices: sessions.ok ? groupSessions(sessions.value) : s.devices,
      passkeys: passkeys.ok ? passkeys.value : s.passkeys,
      factors: score.ok ? score.value.factors : s.factors,
      activity: activity.ok
        ? activity.value.events.map((e) => ({ id: e.id, what: e.description, at: e.created_at, country: e.country_code ?? null, address: null }))
        : s.activity,
    }))
  }, [])
  useEffect(() => { void reload() }, [reload])
  return { state, setState, error, reload, sessionList, localPc }
}

/** Board words for each error code a B14 command returns. */
export const ERRORS: Record<string, string> = {
  wrong_password: 'Incorrect password.',
  invalid_code: 'Invalid code. Try again.',
  // Any rule of the server's password policy (its minimum length can be above core's 12), so no number here.
  weak_password: 'This password doesn’t meet the password rules. Use a longer one.',
  same_password: 'Choose a password you haven’t used for this account.',
  name_empty: 'A name can’t be empty.',
  name_too_long: 'Use 64 characters or fewer.',
  invalid_email: 'Enter a valid email address.',
  email_in_use: 'This email is already in use.',
  rate_limited: 'Too many attempts. Wait a minute, then try again.',
  vault_locked: 'Unlock first, then change your password.',
  opaque_unavailable: 'Sign out and sign in again once, then try this again.',
  network: 'Beebeeb can’t reach the server. Check the connection and try again.',
  server_unsupported: 'This isn’t available in this version of Beebeeb yet.',
  second_factor_required: 'Enter a code from your authenticator app, or a backup code.',
  link_invalid: 'This link doesn’t work anymore. Use the link in the newest email, or start again.',
  link_needed: 'Paste the link from the email to undo the deletion.',
  confirmation_required: 'Enter your password to confirm.',
  // The password change runs sign-up's gate (core v0.2.8): DW-A38's words for a breached password.
  password_breached: BREACHED,
  breach_check_blocked: BREACH_BLOCKED,
  breach_check_missing: 'We could not check this password against known breaches. Try again.',
  policy_unavailable: 'Beebeeb can’t read the password rules from the server right now. Nothing was changed.',
  password_required: 'Enter your password to confirm.',
  // Passkeys: no board frame names these; the words follow the board's voice.
  passkey_rp_mismatch: 'The server asked for a passkey for another site, so Beebeeb stopped. Nothing was changed.',
  passkey_cancelled: 'Windows Hello was cancelled. Nothing was changed.',
  passkey_unavailable: 'Passkeys aren’t available on this PC.',
  passkey_step_up_failed: 'Beebeeb couldn’t confirm your password. Nothing was changed.',
}

/** The codes that are about the new password, so the error shows under that field. */
export const NEW_PASSWORD_ERRORS = ['weak_password', 'same_password', 'password_breached', 'breach_check_blocked', 'breach_check_missing']
export function errorText(code: string): string {
  return ERRORS[code] ?? 'Something went wrong. Nothing was changed.'
}

/** Said under a successful session list that has no other device. Not an error, and not a device. */
export const OTHER_DEVICES_NOTE = 'Other devices appear here after they sign in on this account.'

/** The three reads an empty list uses to describe this PC. Null fields have not answered yet. */
export interface DevicesPaneInput {
  /** `account_session_list`. Null until that command has answered. */
  sessions: CommandResult<AccountSessionList> | null
  /** `sync_status`. */
  sync: CommandResult<Pick<SyncStatus, 'logged_in'>> | null
  /** `app_version`. */
  version: CommandResult<unknown> | null
  /** `desktop_platform`, so the row can name the OS without inventing one. */
  platform: CommandResult<DesktopPlatform> | null
}

export type DevicesPaneModel =
  | { kind: 'pending' }
  | { kind: 'failed'; message: string }
  | { kind: 'empty'; thisPc: DeviceGroup | null; note: string }
  | { kind: 'rows'; devices: DeviceGroup[] }

function platformOs(platform: DevicesPaneInput['platform']): string {
  if (!platform?.ok) return ''
  return OS[platform.value] ?? ''
}

function versionText(version: DevicesPaneInput['version']): string | null {
  if (!version?.ok) return null
  return appVersionFields(version.value)?.version ?? null
}

/** This PC from local commands. No session id: nothing here can be revoked as if the server had listed it. */
function thisPcRow(input: DevicesPaneInput): DeviceGroup {
  const version = versionText(input.version)
  return {
    session_ids: [],
    name: 'This PC',
    os: platformOs(input.platform),
    apps: version ? ['Drive', version] : ['Drive'],
    last_active_at: new Date().toISOString(),
    country_code: null,
    is_current: true,
  }
}

/**
 * What Devices and sessions draws. A refused or unreachable list is the error, not zero rows. An empty success shows
 * this PC only when `sync_status` says this PC is signed in — otherwise there is no device to show.
 */
export function devicesPaneModel(input: DevicesPaneInput): DevicesPaneModel {
  if (!input.sessions) return { kind: 'pending' }
  if (!input.sessions.ok) {
    return { kind: 'failed', message: input.sessions.unsupported ? commandUnavailableLabel('account_session_list') : ERRORS.network }
  }
  const devices = groupSessions(input.sessions.value)
  if (devices.length > 0) return { kind: 'rows', devices }
  const signedIn = !!input.sync?.ok && input.sync.value.logged_in === true
  return { kind: 'empty', thisPc: signedIn ? thisPcRow(input) : null, note: OTHER_DEVICES_NOTE }
}

/**
 * Run a command. A caller that still passes a `fake` (call sites outside the account panes) gets it in a plain browser;
 * without one every answer comes from the command, so a browser shows the host's honest refusal instead of example data.
 */
export async function run<T>(name: string, args: Record<string, unknown>, fake?: T): Promise<{ ok: true; value: T } | { ok: false; code: string }> {
  if (fake !== undefined && !hasTauri()) return { ok: true, value: fake }
  const r = await command<T>(name, args)
  return r.ok ? { ok: true, value: r.value } : { ok: false, code: r.reason }
}

/**
 * Copy, then clear the clipboard after 60 seconds if it still holds what we put there (the web's rule). Resolves to
 * whether the clipboard took it, so the screen never says "Copied" when nothing was copied.
 */
export async function copyFor60s(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    return false
  }
  window.setTimeout(async () => {
    try { if ((await navigator.clipboard.readText()) === text) await navigator.clipboard.writeText('') } catch { /* nothing to clear */ }
  }, 60_000)
  return true
}

/** What a copy button says after a click: the clipboard took it, or it did not. */
export const COPY_DONE = 'Copied. The clipboard clears after 60 seconds.'
export const COPY_FAILED = 'Couldn’t copy. Select the text and copy it yourself.'
