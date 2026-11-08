// B14 data layer: every read and write goes through command() (never the shared API, never a key or token).
// In a plain browser (headless frames, `bun run dev`) there is no IPC bridge: the board's fixtures stand in.
import { useCallback, useEffect, useState } from 'react'
import { command, type AccountProfile, type AccountActivity, type AccountSessionList, type SecurityScore } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import {
  FIXTURE_ACCOUNT, FIXTURE_ACTIVITY, FIXTURE_DEVICES, FIXTURE_FACTORS, FIXTURE_PASSKEYS, FIXTURE_PRODUCTS,
  type ActivityEvent, type DeviceGroup, type Factor, type Passkey, type ProductRow,
} from '../../../source/fixtures/account'

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

function fixtureState(scenario: Scenario): AccountState {
  const passkeys = scenario === 'no-passkey' || scenario === 'no-passkeys-at-all' ? [] : FIXTURE_PASSKEYS
  return {
    name: FIXTURE_ACCOUNT.name,
    email: FIXTURE_ACCOUNT.email,
    emailVerified: FIXTURE_ACCOUNT.email_verified,
    passwordChangedAt: FIXTURE_ACCOUNT.password_changed_at,
    totp: { on: true, addedAt: FIXTURE_ACCOUNT.totp.added_at, lastUsedAt: FIXTURE_ACCOUNT.totp.last_used_at, codesLeft: FIXTURE_ACCOUNT.totp.codes_left, codesTotal: 8 },
    activityLogging: scenario !== 'logging-off',
    products: FIXTURE_PRODUCTS,
    devices: FIXTURE_DEVICES,
    passkeys,
    factors: FIXTURE_FACTORS.map((f) => (f.key === 'passkey_added' ? { ...f, satisfied: passkeys.length > 0 } : f)),
    activity: scenario === 'logging-off' ? [] : FIXTURE_ACTIVITY,
  }
}

const OS: Record<string, string> = { windows: 'Windows', macos: 'macOS', linux: 'Linux', ios: 'iOS', android: 'Android', web: 'Web', cli: 'Command line' }

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

/** Reads every account fact once and again on `reload()` (the checkup checks again each time it opens). */
export function useAccount(scenario: Scenario = 'default') {
  const [state, setState] = useState<AccountState>(() => fixtureState(scenario))
  const [error, setError] = useState<string | null>(null)
  const reload = useCallback(async () => {
    if (!hasTauri()) return
    const [profile, name, sessions, score, activity, logging, passkeys] = await Promise.all([
      command<AccountProfile>('account_profile'),
      command<{ display_name: string | null }>('account_display_name'),
      command<AccountSessionList>('account_session_list'),
      command<SecurityScore>('account_security_score'),
      command<AccountActivity>('account_activity'),
      command<{ on: boolean }>('account_activity_logging'),
      command<Passkey[]>('account_passkeys_list'),
    ])
    const firstError = [profile, sessions, score].find((r) => !r.ok)
    setError(firstError && !firstError.ok ? firstError.reason : null)
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
      passkeys: passkeys.ok ? passkeys.value : [],
      factors: score.ok ? score.value.factors : s.factors,
      activity: activity.ok
        ? activity.value.events.map((e) => ({ id: e.id, what: e.description, at: e.created_at, country: e.country_code ?? null, address: null }))
        : s.activity,
    }))
  }, [])
  useEffect(() => { void reload() }, [reload])
  return { state, setState, error, reload }
}

/** Board words for each error code a B14 command returns. */
export const ERRORS: Record<string, string> = {
  wrong_password: 'Incorrect password.',
  invalid_code: 'Invalid code. Try again.',
  weak_password: 'Use at least 12 characters.',
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
}
export function errorText(code: string): string {
  return ERRORS[code] ?? 'Something went wrong. Nothing was changed.'
}

/** Run a command; in a plain browser the fixture answer `fake` stands in. */
export async function run<T>(name: string, args: Record<string, unknown>, fake: T): Promise<{ ok: true; value: T } | { ok: false; code: string }> {
  if (!hasTauri()) return { ok: true, value: fake }
  const r = await command<T>(name, args)
  return r.ok ? { ok: true, value: r.value } : { ok: false, code: r.reason }
}

/** Copy, then clear the clipboard after 60 seconds if it still holds what we put there (the web's rule). */
export async function copyFor60s(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    window.setTimeout(async () => {
      try { if ((await navigator.clipboard.readText()) === text) await navigator.clipboard.writeText('') } catch { /* nothing to clear */ }
    }, 60_000)
  } catch { /* clipboard refused: nothing copied */ }
}
