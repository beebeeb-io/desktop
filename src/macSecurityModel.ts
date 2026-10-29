/**
 * Pure logic behind Settings › Security (task 1683 slice 5; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8
 * "Security" and §10.2 #29; hi-fi section 4, "Security (new)").
 *
 * The copy is the Windows shell's (`SecurityView`, `DevicesView`, `ActivityView`), kept, with two changes the spec makes:
 * the Devices subtitle no longer promises a revoke that has no control (#29), and no word is a developer word (§10.1).
 * Nothing here touches React or Tauri, so `tests/macSecurityModel.test.ts` can pin every sentence.
 *
 * Dates are written in English on any Mac ("3 Sep", "Today 09:12"), like the plan line's "renews 14 Nov 2026": this app's
 * words are English, and a Dutch month name inside an English sentence reads as a mistake. Numbers (sizes) follow the Mac.
 */
import type { AccountProfile, AccountSession, ActivityFeedEvent, ClientDevice } from './desktopApi'
import { ageOf } from './relativeTime'
import { SECURITY_SETTINGS_URL } from './settingsConstants'

export { SECURITY_SETTINGS_URL }

/**
 * The "Active sessions" heading, which takes focus once a revoke has removed the row (or the button) that had it: a
 * heading is where a list starts, and it stays when every row in it goes (PR #140 review, P3).
 */
export const SESSIONS_HEADING_ID = 'ms-security-sessions'

/** The Devices subtitle (spec #29): no revoke, because no control to revoke a device exists. */
export const DEVICES_FOOTNOTE = 'Every device holding a copy of your vault key, and the sync sessions running on each.'

/** "Recent account activity" shows the last ten events (spec §7.8). */
export const ACTIVITY_LIMIT = 10

// ── The authenticator row ───────────────────────────────────────────────────

export type TotpState = 'on' | 'off' | 'unknown' | 'checking'

export const TOTP_LABEL = 'Authenticator app (TOTP)'

/**
 * The most often the window's own looks at the account (when it is focused again) ask the server for the authenticator's
 * state: a minute. Opening the section and "Try again" always ask.
 */
export const PROFILE_RECHECK_MS = 60_000

/** The one inline alert when the account could not be asked (one failure, one surface; Try again asks again). */
export const TOTP_CHECK_FAILED = 'Couldn’t check your authenticator app'

/**
 * The state of the authenticator app, from the account's own `totp_enabled` (the profile). Not from the security score: its
 * `two_factor_enabled` factor is an authenticator OR a passkey and ignores whether the authenticator is on, so a row built on
 * it can say On when it is Off (PR #140 review, P2-5). `unknown` when the profile does not say, so the row never claims On
 * or Off.
 */
export function totpState(profile: Pick<AccountProfile, 'totp_enabled'> | null | undefined): 'on' | 'off' | 'unknown' {
  const enabled = profile?.totp_enabled
  return enabled === true ? 'on' : enabled === false ? 'off' : 'unknown'
}

/**
 * What the row shows for a look at the account: "checking" while the question is out, `failed` when the first look, or one
 * the person asked for, could not be asked (the page shows an alert with Try again, not a row), the account's own answer
 * otherwise. A look at the window that fails changes nothing: the answer on screen stays, and says its age once it is five
 * minutes old (`totpStaleNote`), so it is never shown as current when it is not. The look is always the server's (`account_profile_fresh`), never the copy the session cached at sign-in, which
 * said On for hours after the authenticator was turned off on the web (PR #140 delta review, P2-B).
 */
export function totpRowState(
  look: { status: 'loading' } | { status: 'failed' } | { status: 'ready'; value: Pick<AccountProfile, 'totp_enabled'> },
): TotpState | 'failed' {
  if (look.status === 'loading') return 'checking'
  if (look.status === 'failed') return 'failed'
  return totpState(look.value)
}

/**
 * How old an On or Off may be before the row says so: five minutes (lead ruling on #147: "a security answer shown as current
 * must say its age once it's stale"). The answer on screen was seen, never cached or guessed, but a look at the window that
 * fails leaves the last one standing, and a window left alone for an hour is showing an hour-old answer.
 */
export const TOTP_STALE_AFTER_MS = 5 * 60_000

/**
 * "Last checked 7 minutes ago." for the row's secondary text, once the answer is `TOTP_STALE_AFTER_MS` old, else null. The
 * wording is the Mac's (`agoLabel`: spelled out, as the sessions say it) over the shared arithmetic (`relativeTime.ts`).
 */
export function totpStaleNote(answeredAt: number, now: number): string | null {
  // A clock put back (sleep, a manual change) puts `now` before the answer: nothing is current about that, and its age
  // cannot be said in minutes (review P3-b).
  if (now < answeredAt) return 'Last checked a while ago.'
  if (now - answeredAt < TOTP_STALE_AFTER_MS) return null
  const ago = agoLabel(new Date(answeredAt).toISOString(), now)
  return ago === null ? null : `Last checked ${ago}.`
}

export function totpLabel(state: TotpState): string {
  return state === 'on' ? `${TOTP_LABEL} · On` : state === 'off' ? `${TOTP_LABEL} · Off` : TOTP_LABEL
}

export function totpHint(state: TotpState): string {
  switch (state) {
    case 'checking':
      return 'Checking…'
    case 'on':
      return 'A second factor is required when you sign in. Manage or reset it on beebeeb.io.'
    case 'off':
      return 'Add a second factor so a stolen password alone can’t open your vault. Set it up on beebeeb.io.'
    default:
      return 'Manage two-factor authentication on beebeeb.io.'
  }
}

export function totpAction(state: TotpState): string {
  return state === 'off' ? 'Set up on beebeeb.io' : 'Manage on beebeeb.io'
}

// ── Time, said in words ─────────────────────────────────────────────────────

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const

function parse(iso: string | null | undefined): Date | null {
  if (!iso) return null
  const t = Date.parse(iso)
  return Number.isNaN(t) ? null : new Date(t)
}

/** "3 Sep" in the year of `now`, "3 Sep 2025" in any other. */
function dayLabel(date: Date, now: number): string {
  const day = `${date.getDate()} ${MONTHS[date.getMonth()]}`
  return date.getFullYear() === new Date(now).getFullYear() ? day : `${day} ${date.getFullYear()}`
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/**
 * "just now", "5 minutes ago", "2 hours ago", "3 days ago", and after a month a date. Null when the time is missing or
 * unreadable. A clock that is ahead of ours reads as "just now".
 */
export function agoLabel(iso: string | null | undefined, now: number): string | null {
  const date = parse(iso)
  if (!date) return null
  const age = ageOf(date, now, 30)
  switch (age.unit) {
    case 'now':
      return 'just now'
    case 'minute':
      return `${plural(age.count, 'minute', 'minutes')} ago`
    case 'hour':
      return `${plural(age.count, 'hour', 'hours')} ago`
    case 'day':
      return `${plural(age.count, 'day', 'days')} ago`
    case 'date':
      return dayLabel(age.date, now)
  }
}

const pad = (n: number) => String(n).padStart(2, '0')

/** "Today 09:12", "Yesterday 18:03", "3 Sep 07:05". Null when the time is missing or unreadable. */
function whenLabel(iso: string | null | undefined, now: number): string | null {
  const date = parse(iso)
  if (!date) return null
  const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`
  const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  const days = Math.round((startOfDay(new Date(now)) - startOfDay(date)) / 86_400_000)
  if (days <= 0) return `Today ${clock}`
  if (days === 1) return `Yesterday ${clock}`
  return `${dayLabel(date, now)} ${clock}`
}

// ── Sessions ────────────────────────────────────────────────────────────────

export interface SessionRow {
  id: string
  title: string
  hint: string
  current: boolean
}

export function sessionRow(session: AccountSession, now: number): SessionRow {
  const name = session.device_name?.trim() || 'Unknown device'
  if (session.is_current) return { id: session.id, title: `${name} · This session`, hint: 'Active now', current: true }
  const ago = agoLabel(session.last_active_at ?? session.created_at, now)
  return { id: session.id, title: name, hint: ago ? `Active ${ago}` : 'Last active time unknown', current: false }
}

// ── Devices ─────────────────────────────────────────────────────────────────

export interface DeviceRow {
  id: string
  title: string
  hint: string
}

function platformName(platform: string | null | undefined): string {
  const p = (platform ?? '').trim()
  if (!p) return 'Unknown platform'
  switch (p.toLowerCase()) {
    case 'macos':
    case 'darwin':
      return 'macOS'
    case 'windows':
    case 'win32':
      return 'Windows'
    case 'linux':
      return 'Linux'
    case 'ios':
      return 'iOS'
    case 'android':
      return 'Android'
    default:
      return p
  }
}

export function deviceRow(device: ClientDevice, now: number): DeviceRow {
  const title = [device.hostname, platformName(device.platform), device.bb_version ? `v${device.bb_version}` : null].filter(Boolean).join(' · ')
  const seen = agoLabel(device.last_seen, now)
  const registered = parse(device.created_at)
  const parts = [
    seen ? `last seen ${seen}` : 'last seen time unknown',
    registered ? `registered ${dayLabel(registered, now)}` : null,
    device.session_count > 0 ? plural(device.session_count, 'sync session', 'sync sessions') : null,
  ].filter(Boolean)
  return { id: device.id, title, hint: parts.join(' · ') }
}

// ── Recent account activity ─────────────────────────────────────────────────

export interface ActivityRow {
  id: string
  title: string
  /** Null when the event's time could not be read. */
  when: string | null
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
/** True when every comma-separated part is an id: the server sets those as subjects where it never sees a file name. */
function looksLikeIds(subject: string): boolean {
  const parts = subject.split(',').map((p) => p.trim()).filter(Boolean)
  return parts.length > 0 && parts.every((p) => UUID.test(p))
}

const TYPE_LABELS: Record<string, string> = {
  'file.trash': 'Moved an item to trash',
  'file.downloaded': 'Downloaded an item',
  'auth.logout': 'Signed out',
  'user.logout': 'Signed out',
}

function humanizeType(type: string): string {
  const raw = (type || '').replace(/[._]+/g, ' ').trim()
  return raw ? raw.charAt(0).toUpperCase() + raw.slice(1) : 'Activity'
}

/** A person's words for an audit event, and never a raw id: known types first, a bulk trash counted, a name, the type. */
function activityTitle(event: ActivityFeedEvent): string {
  if (event.type === 'file.trash.bulk') {
    const count = (event.subject ?? '').split(',').map((s) => s.trim()).filter(Boolean).length
    return count > 1 ? `Moved ${count} items to trash` : 'Moved an item to trash'
  }
  if (event.type in TYPE_LABELS) return TYPE_LABELS[event.type]
  const subject = event.subject?.trim()
  if (subject && !looksLikeIds(subject)) return subject
  return humanizeType(event.type)
}

/** The last `limit` events, newest first; an event whose time cannot be read is kept, after the ones that can. */
export function activityRows(events: ActivityFeedEvent[], now: number, limit: number = ACTIVITY_LIMIT): ActivityRow[] {
  const time = (event: ActivityFeedEvent) => parse(event.created_at)?.getTime() ?? Number.NEGATIVE_INFINITY
  return [...events]
    .sort((a, b) => time(b) - time(a))
    .slice(0, limit)
    .map((event) => ({ id: event.id, title: activityTitle(event), when: whenLabel(event.created_at, now) }))
}
