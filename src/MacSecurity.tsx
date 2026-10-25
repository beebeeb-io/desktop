/**
 * Settings › Security (task 1683 slice 5; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8, row "Security"; hi-fi
 * `design/hifi/macos-two-surface.html` section 4, "Security (new)"): what the Windows shell kept in Security, Devices and
 * Activity, as the same grouped rows the other sections use.
 *
 * Four reads, none of them blocking another: the account profile (only for the state of the authenticator app), the active
 * sessions, the devices and the last ten events. A group that did not load is ONE inline alert with Try again and leaves the other groups
 * alone; the authenticator row never claims On or Off when it could not read the score. This app has no control to turn
 * two-factor on or off, so the row says where to do that (the web app) and offers nothing it cannot do. Revoking a session
 * asks first, in the same sheet Sign out of this Mac uses; a revoke that fails is a toast (a one-off action).
 *
 * A locked vault shows no account data and asks for none. The copy is in `macSecurityModel.ts`.
 *
 * The component is a plain function declaration that reads only its props, hooks and the bindings imported above, so
 * `tests/macSecurity.test.tsx` can execute it with the component harness.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  accountActivityFeed,
  accountDevices,
  accountRevokeOtherSessions,
  accountProfileFresh,
  accountRevokeSession,
  accountSessionList,
  commandUnavailableLabel,
  openUrl,
  type AccountProfile,
  type AccountSession,
  type ActivityFeedEvent,
  type ClientDevice,
} from './desktopApi'
import {
  ACTIVITY_LIMIT,
  activityRows,
  DEVICES_FOOTNOTE,
  deviceRow,
  SECURITY_SETTINGS_URL,
  PROFILE_RECHECK_MS,
  SESSIONS_HEADING_ID,
  sessionRow,
  TOTP_CHECK_FAILED,
  TOTP_STALE_AFTER_MS,
  totpAction,
  totpHint,
  totpLabel,
  totpRowState,
  totpStaleNote,
} from './macSecurityModel'
import { focusById } from './focusById'
import { ConfirmSheet } from './MacSettings'
import { Btn, Note, SettingRow, SettingsGroup, SettingsIcon } from './macSettingsParts'
import { useToast } from './windows/ui'

type Load<T> = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; value: T }
/** The authenticator's look: the answer and when it arrived (its age is said once it is five minutes old). */
type ProfileState = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; value: AccountProfile; at: number }
type Ask = { kind: 'one'; id: string; name: string } | { kind: 'others'; count: number } | null

export function SecurityTab({ unlocked }: { unlocked: boolean }) {
  const { showToast } = useToast()
  const [profile, setProfile] = useState<ProfileState>({ status: 'loading' })
  // Only here to ask for a render: the age of an answer on screen must not stand still, and a failed look that sets no state still needs one.
  const [, setTick] = useState(0)
  const [sessions, setSessions] = useState<Load<AccountSession[]>>({ status: 'loading' })
  const [devices, setDevices] = useState<Load<ClientDevice[]>>({ status: 'loading' })
  const [activity, setActivity] = useState<Load<ActivityFeedEvent[]>>({ status: 'loading' })
  const [asking, setAsking] = useState<Ask>(null)
  const [busy, setBusy] = useState(false)

  // `quiet` reads again without blanking the group first: after a revoke the rows are replaced, not flashed.
  // The authenticator row is the account's answer, read from the server (not the session's cache; review P2-B) when the page
  // opens, when the person asks again (Try again), and when the window is looked at again, which is at most once a minute
  // after an ANSWER (review P3-4; a failed look learned nothing, so it holds nothing back: P3-d; and a clock put back holds
  // nothing back either: P3-b). Only the latest look may answer. A look the person asked for, or the first, that fails is the
  // alert; a look at the window that fails changes nothing on a row that has an answer: it was seen, and an alert would come
  // up under a person who asked for nothing. It still asks for a render, because the answer's age may just have come due
  // (P3-a); and on a row with no answer yet (the look it took over was out) it is the alert, never "Checking…" for good (P3-c).
  const latestProfile = useRef(0)
  const lastProfileLook = useRef(0)
  const loadProfile = useCallback(async (how: 'asked' | 'looked' = 'asked') => {
    const since = Date.now() - lastProfileLook.current
    if (how === 'looked' && since >= 0 && since < PROFILE_RECHECK_MS) return
    const mine = ++latestProfile.current
    if (how === 'asked') setProfile({ status: 'loading' })
    const result = await accountProfileFresh()
    if (mine !== latestProfile.current) return
    if (result.ok) {
      const at = Date.now()
      lastProfileLook.current = at
      setProfile({ status: 'ready', value: result.value, at })
      return
    }
    setProfile((current) => (how === 'asked' || current.status !== 'ready' ? { status: 'failed' } : current))
    setTick((n) => n + 1)
  }, [])
  const loadSessions = useCallback(async (quiet = false) => {
    if (!quiet) setSessions({ status: 'loading' })
    const result = await accountSessionList()
    setSessions(result.ok ? { status: 'ready', value: result.value.sessions ?? [] } : { status: 'failed' })
  }, [])
  const loadDevices = useCallback(async () => {
    setDevices({ status: 'loading' })
    const result = await accountDevices()
    setDevices(result.ok ? { status: 'ready', value: result.value.devices ?? [] } : { status: 'failed' })
  }, [])
  const loadActivity = useCallback(async (quiet = false) => {
    if (!quiet) setActivity({ status: 'loading' })
    const result = await accountActivityFeed(1, ACTIVITY_LIMIT)
    setActivity(result.ok ? { status: 'ready', value: result.value.events ?? [] } : { status: 'failed' })
  }, [])

  useEffect(() => {
    if (!unlocked) return
    void loadProfile()
    void loadSessions()
    void loadDevices()
    void loadActivity()
  }, [unlocked, loadProfile, loadSessions, loadDevices, loadActivity])

  // The answer's age is said once it is five minutes old (`totpStaleNote`): the first render is asked for at the moment it goes
  // stale (a wake from sleep runs a late timer at once), then a minute's tick lets the age grow while nobody looks (P3-a).
  const answeredAt = profile.status === 'ready' ? profile.at : null
  useEffect(() => {
    if (!unlocked || answeredAt === null) return
    const bump = () => setTick((n) => n + 1)
    let every: number | undefined
    const first = window.setTimeout(() => {
      bump()
      every = window.setInterval(bump, 60_000)
    }, Math.max(0, answeredAt + TOTP_STALE_AFTER_MS - Date.now()))
    return () => {
      window.clearTimeout(first)
      if (every !== undefined) window.clearInterval(every)
    }
  }, [unlocked, answeredAt])

  // A change made on the web while this window was in the background shows the moment the window is looked at again.
  useEffect(() => {
    if (!unlocked) return
    const onFocus = () => void loadProfile('looked')
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [unlocked, loadProfile])

  const revokeOne = async (id: string) => {
    if (busy) return
    setBusy(true)
    const result = await accountRevokeSession(id)
    setBusy(false)
    setAsking(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t sign out that session',
        message: result.unsupported ? commandUnavailableLabel('account_revoke_session') : result.reason,
      })
      return
    }
    showToast({ variant: 'success', title: 'Session signed out', message: 'That device will need to sign in again.' })
    await Promise.all([loadSessions(true), loadActivity(true)])
    // The Revoke button that opened the question left with its row: focus goes to where the list starts.
    focusById(SESSIONS_HEADING_ID)
  }

  const revokeOthers = async () => {
    if (busy) return
    setBusy(true)
    const result = await accountRevokeOtherSessions()
    setBusy(false)
    setAsking(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t sign out other sessions',
        message: result.unsupported ? commandUnavailableLabel('account_revoke_other_sessions') : result.reason,
      })
      return
    }
    const count = result.value?.revoked ?? 0
    showToast({
      variant: 'success',
      title: count === 1 ? 'Signed out 1 other session' : `Signed out ${count} other sessions`,
      message: 'Those devices will need to sign in again.',
    })
    await Promise.all([loadSessions(true), loadActivity(true)])
    // The button that opened the question left with the rows it ended.
    focusById(SESSIONS_HEADING_ID)
  }

  if (!unlocked) {
    return (
      <SettingsGroup>
        <SettingRow name="locked" tall label="Your vault is locked" hint="Unlock it to see your sessions, devices and recent activity." />
      </SettingsGroup>
    )
  }

  const now = Date.now()
  const totp = totpRowState(profile)
  const staleNote = profile.status === 'ready' && (totp === 'on' || totp === 'off') ? totpStaleNote(profile.at, now) : null

  const loading = (name: string) => <SettingRow name={name} label="Loading…" />
  const failed = (surface: string, title: string, retry: () => void) => (
    <Note kind="alert" surface={surface} title={title} actions={<Btn onClick={retry}>Try again</Btn>} />
  )

  let sessionRows = null
  if (sessions.status === 'loading') sessionRows = loading('sessions-loading')
  else if (sessions.status === 'failed') sessionRows = failed('security-sessions', 'Couldn’t load your active sessions', () => void loadSessions())
  else if (sessions.value.length === 0) {
    sessionRows = <SettingRow name="sessions-empty" tall label="No active sessions" hint="Nothing is signed in to your account right now." />
  } else {
    const others = sessions.value.filter((session) => !session.is_current).length
    sessionRows = (
      <>
        {sessions.value.map((session) => {
          const row = sessionRow(session, now)
          return (
            <SettingRow
              key={row.id}
              name={`session-${row.id}`}
              label={row.title}
              hint={row.hint}
              control={
                row.current ? null : (
                  <Btn ariaLabel={`Revoke ${row.title}`} disabled={busy} onClick={() => setAsking({ kind: 'one', id: row.id, name: row.title })}>
                    Revoke
                  </Btn>
                )
              }
            />
          )
        })}
        {others > 0 ? (
          <SettingRow
            name="others"
            tall
            label="Sign out everywhere else"
            hint="Ends every session except this one. Those devices will need to sign in again."
            control={
              <Btn disabled={busy} onClick={() => setAsking({ kind: 'others', count: others })}>
                Sign out all other sessions
              </Btn>
            }
          />
        ) : null}
      </>
    )
  }

  let deviceRows = null
  if (devices.status === 'loading') deviceRows = loading('devices-loading')
  else if (devices.status === 'failed') deviceRows = failed('security-devices', 'Couldn’t load your devices', () => void loadDevices())
  else if (devices.value.length === 0) deviceRows = <SettingRow name="devices-empty" label="No devices registered yet" />
  else {
    deviceRows = (
      <>
        {devices.value.map((device) => {
          const row = deviceRow(device, now)
          return <SettingRow key={row.id} name={`device-${row.id}`} tall label={row.title} hint={row.hint} />
        })}
      </>
    )
  }

  let activityRowsView = null
  if (activity.status === 'loading') activityRowsView = loading('activity-loading')
  else if (activity.status === 'failed') activityRowsView = failed('security-activity', 'Couldn’t load recent activity', () => void loadActivity())
  else if (activity.value.length === 0) activityRowsView = <SettingRow name="activity-empty" label="No recent activity." />
  else {
    activityRowsView = (
      <>
        {activityRows(activity.value, now).map((row) => (
          <SettingRow key={row.id} name={`event-${row.id}`} label={row.title} hint={row.when ?? undefined} />
        ))}
      </>
    )
  }

  const askedName = asking?.kind === 'one' ? asking.name : ''

  return (
    <>
      <SettingsGroup>
        {totp === 'failed' ? (
          failed('security-profile', TOTP_CHECK_FAILED, () => void loadProfile())
        ) : (
          <SettingRow
            name="totp"
            tall
            label={totpLabel(totp)}
            hint={staleNote ? `${totpHint(totp)} ${staleNote}` : totpHint(totp)}
            control={
              <Btn onClick={() => void openUrl(SECURITY_SETTINGS_URL)}>
                <SettingsIcon name="external" size={13} />
                {totpAction(totp)}
              </Btn>
            }
          />
        )}
      </SettingsGroup>

      <SettingsGroup title="Active sessions" headingId={SESSIONS_HEADING_ID}>{sessionRows}</SettingsGroup>

      <SettingsGroup title="Devices" footnote={devices.status === 'ready' ? DEVICES_FOOTNOTE : undefined}>
        {deviceRows}
      </SettingsGroup>

      <SettingsGroup title="Recent account activity">{activityRowsView}</SettingsGroup>

      <ConfirmSheet
        open={asking?.kind === 'one'}
        title="Sign out this session?"
        confirmLabel="Sign out"
        danger
        onCancel={() => setAsking(null)}
        onConfirm={() => asking?.kind === 'one' && void revokeOne(asking.id)}
      >
        {`${askedName} will need to sign in again.`}
      </ConfirmSheet>
      <ConfirmSheet
        open={asking?.kind === 'others'}
        title="Sign out everywhere else?"
        confirmLabel="Sign out others"
        danger
        onCancel={() => setAsking(null)}
        onConfirm={() => void revokeOthers()}
      >
        Every other session ends. Those devices will need to sign in again.
      </ConfirmSheet>
    </>
  )
}
