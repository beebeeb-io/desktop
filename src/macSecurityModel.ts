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
import type { AccountSession, ActivityFeedEvent, ClientDevice, SecurityScore } from './desktopApi'

/** Where two-factor is set up and managed: the web app has the controls, this app has none (so none is promised). */
export const SECURITY_SETTINGS_URL = 'https://app.beebeeb.io/settings/security'

/** The Devices subtitle (spec #29): no revoke, because no control to revoke a device exists. */
export const DEVICES_FOOTNOTE = 'Every device holding a copy of your vault key, and the sync sessions running on each.'

/** "Recent account activity" shows the last ten events (spec §7.8). */
export const ACTIVITY_LIMIT = 10

// ── The authenticator row ───────────────────────────────────────────────────

export type TotpState = 'on' | 'off' | 'unknown'

export const TOTP_LABEL = 'Authenticator app (TOTP)'

/**
 * The state of two-factor, read from the security score's factors: the only honest source this app has. `unknown` when
 * the score is unavailable or none of its factors speaks about a second factor, so the row never claims On or Off.
 */
export function totpState(score: Pick<SecurityScore, 'factors'> | null | undefined): TotpState {
  const factor = score?.factors?.find((f) => f.key === 'two_factor_enabled' || f.key === 'totp_enabled')
  if (!factor) return 'unknown'
  return factor.satisfied ? 'on' : 'off'
}

export function totpLabel(state: TotpState): string {
  return state === 'on' ? `${TOTP_LABEL} · On` : state === 'off' ? `${TOTP_LABEL} · Off` : TOTP_LABEL
}

export function totpHint(state: TotpState): string {
  switch (state) {
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
  const seconds = Math.round((now - date.getTime()) / 1000)
  if (seconds < 45) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${plural(minutes, 'minute', 'minutes')} ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${plural(hours, 'hour', 'hours')} ago`
  const days = Math.round(hours / 24)
  if (days < 30) return `${plural(days, 'day', 'days')} ago`
  return dayLabel(date, now)
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
