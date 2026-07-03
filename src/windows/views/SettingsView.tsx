import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  accountNotificationPreferences,
  accountUpdateNotificationPreferences,
  appActivitySnapshot,
  checkForDesktopUpdatesNow,
  command,
  commandUnavailableLabel,
  formatBytes,
  installDesktopChannelDowngrade,
  openUrl,
  type AppActivitySnapshot,
  type DesktopConfig,
  type DesktopTheme,
  type FinderInstallState,
  type NotificationPreferenceValues,
  type StorageSummary,
  type SyncStatus,
  DEFAULT_CONFIG,
} from '../../desktopApi'
import {
  ALWAYS_ACCESSIBLE_SETTINGS,
  availableSettingsSections,
  defaultSettingsPage,
  settingLabel,
  type SettingsNavId,
} from '../settingsNavigation'
import {
  buildUpdateCheckViewModel,
  releaseChannelLabel,
  stateFromManualUpdateResult,
  type ManualUpdateCheckState,
} from '../updateCheckViewModel'
import {
  ACCOUNT_NOTIFICATION_PREF_META,
  DESKTOP_NOTIFICATION_PREF_META,
  type AccountNotificationPrefKey,
  type DesktopNotificationPrefKey,
} from '../notificationPreferenceMeta'
import { T, Card, Chip, NavIcon, PageHeader, PrimaryBtn, Skeleton } from '../ui'
import { useRegionCity } from '../useRegion'
import {
  CACHE_LIMIT_OPTIONS,
  buildCacheLimitView,
  normalizeCacheLimitBytes,
  normalizeThemePreference,
} from '../advancedSettingsModel'
import { setDesktopThemePreference } from '../theme'

type ShellIntegrationState = FinderInstallState

interface SettingsViewProps {
  status: SyncStatus | null
  onOpenSignIn: () => void
}

interface SyncRow {
  label: string
  hint: string
  locked?: boolean
  value?: string
  configKey?: keyof DesktopConfig
}

const SYNC_ROWS: SyncRow[] = [
  {
    label: 'Keep files encrypted end-to-end',
    hint: 'This cannot be disabled. Every byte on disk is ciphertext.',
    locked: true,
  },
  {
    label: 'Sync on metered connections',
    hint: 'When off, Beebeeb waits for Wi-Fi or Ethernet.',
  },
  {
    label: 'Upload rate limit',
    hint: 'Auto = adapt to available bandwidth',
    value: 'Auto',
    configKey: 'upload_kbps_limit',
  },
  {
    label: 'Download rate limit',
    hint: 'Auto = adapt to available bandwidth',
    value: 'Auto',
    configKey: 'download_kbps_limit',
  },
  {
    label: 'Files on Demand (online-only by default)',
    hint: 'Placeholders appear in File Explorer. Open a file to download.',
  },
  {
    label: 'Show sync overlays in File Explorer',
    hint: 'The check, cloud, and spinner icons on your files.',
  },
]

type ReleaseChannelValue = NonNullable<DesktopConfig['release_channel']>

const RELEASE_CHANNELS: Array<{ value: ReleaseChannelValue; label: string; hint: string }> = [
  { value: 'stable', label: 'Stable', hint: 'Regular releases after validation.' },
  { value: 'beta', label: 'Beta', hint: 'Earlier builds for release testing.' },
  { value: 'alpha', label: 'Alpha', hint: 'Earliest builds for rapid feedback.' },
]

const RED = 'var(--red)'
const ACTIVITY_POLL_INTERVAL_MS = 1_500

const THEME_OPTIONS: Array<{ value: DesktopTheme; label: string; hint: string }> = [
  { value: 'light', label: 'Light', hint: 'Use the light desktop palette.' },
  { value: 'dark', label: 'Dark', hint: 'Use the dark desktop palette.' },
  { value: 'system', label: 'System', hint: 'Follow Windows or macOS.' },
]

function formatCpuPercent(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value) || value < 0) return '—'
  if (value < 10) return `${value.toFixed(1)}%`
  return `${Math.round(value)}%`
}

function formatRate(bytesPerSecond: number | null | undefined): string {
  if (bytesPerSecond == null || !Number.isFinite(bytesPerSecond) || bytesPerSecond < 0) return '—'
  return `${formatBytes(bytesPerSecond)}/s`
}

function formatSampleAge(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return 'No local sample'
  if (ms < 1_000) return 'just now'
  const seconds = Math.round(ms / 1_000)
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.round(minutes / 60)
  return `${hours}h ago`
}

function Toggle({ on, busy = false, onChange, label }: { on: boolean; busy?: boolean; onChange: (next: boolean) => void; label: string }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy}
      disabled={busy}
      onClick={() => onChange(!on)}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        border: 'none',
        padding: 2,
        background: on ? T.amberDeep : T.line2,
        cursor: busy ? 'progress' : 'pointer',
        opacity: busy ? 0.6 : 1,
        transition: 'background 150ms ease, opacity 150ms ease',
        display: 'inline-flex',
        alignItems: 'center',
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: 16,
          height: 16,
          borderRadius: '50%',
          background: T.paper,
          transform: on ? 'translateX(16px)' : 'translateX(0)',
          transition: 'transform 150ms ease',
          boxShadow: '0 1px 3px rgba(0,0,0,0.25)',
        }}
      />
    </button>
  )
}

function ErrorBlock({ reason, unsupported, onRetry }: { reason: string; unsupported: boolean; onRetry: () => void }) {
  if (unsupported) {
    return (
      <Card style={{ padding: 22, display: 'flex', alignItems: 'center', gap: 12 }}>
        <NavIcon name="clock" size={16} color={T.ink4} />
        <div>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink2 }}>Not available in this build</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5, marginTop: 2 }}>
            This setting binds to a command that is not wired into the current desktop build.
          </div>
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: 22 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <NavIcon name="external" size={16} color={RED} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Could not load</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5, marginTop: 4, wordBreak: 'break-word' as const }}>
            {reason}
          </div>
        </div>
      </div>
      <div style={{ marginTop: 14 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

function SignedOutSettingsGate({ onOpenSignIn }: { onOpenSignIn: () => void }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', minHeight: 320, padding: '28px 36px', gap: 10 }}>
      <NavIcon name="lock" size={26} color={T.ink3} />
      <div style={{ fontSize: 16, fontWeight: 600, color: T.ink, marginTop: 6 }}>Sign in to manage this setting</div>
      <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 320 }}>
        Sync and notification preferences belong to your encrypted account.
      </div>
      <div style={{ marginTop: 8 }}>
        <PrimaryBtn onClick={onOpenSignIn}>Sign in</PrimaryBtn>
      </div>
    </div>
  )
}

function SettingsSectionShell({ children }: { children: ReactNode }) {
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', minWidth: 0 }}>
      {children}
    </div>
  )
}

function SyncPanel({
  status,
  storage,
  config,
  onConfigChange,
  notice,
}: {
  status: SyncStatus | null
  storage: StorageSummary | null
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  const [freeUpBusy, setFreeUpBusy] = useState(false)
  const [freeUpNote, setFreeUpNote] = useState<string | null>(null)
  const [lastSyncLabel, setLastSyncLabel] = useState('...')

  useEffect(() => {
    if (status?.syncing === 0 && status.engine === 'running') {
      setLastSyncLabel('Just now')
    } else if (status?.engine === 'running') {
      setLastSyncLabel('Syncing...')
    } else {
      setLastSyncLabel('-')
    }
  }, [status?.syncing, status?.engine])

  const freeUp = async () => {
    setFreeUpBusy(true)
    setFreeUpNote(null)
    const result = await command<{ bytes_freed: number }>('free_up_space')
    setFreeUpBusy(false)
    if (result.ok) {
      setFreeUpNote(`Freed ${formatBytes(result.value.bytes_freed)}`)
    } else {
      setFreeUpNote(result.unsupported ? commandUnavailableLabel('free_up_space') : result.reason)
    }
  }

  const synced7d = storage ? formatBytes(storage.used_bytes) : '...'
  const onPc = storage ? formatBytes(storage.pinned_bytes + storage.cache_bytes) : '...'
  const metered = config.metered ?? false
  const filesOnDemand = config.files_on_demand ?? true
  const overlays = config.sync_overlays ?? true

  return (
    <SettingsSectionShell>
      <PageHeader
        title="Sync"
        subtitle="Manage what syncs with this PC and how. Files are encrypted on this device before upload."
        aside={
          <Chip tone={status?.engine === 'running' && status.syncing === 0 ? 'green' : 'amber'}>
            {status?.engine === 'running' && status.syncing === 0 ? 'All devices up to date' : 'Syncing'}
          </Chip>
        }
      />

      <Card style={{ padding: 20, background: `linear-gradient(135deg, ${T.amberBg}, ${T.paper2})`, borderColor: T.amberDeep, marginBottom: 22 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', gap: 20 }}>
          {[
            { k: 'State', v: status?.engine === 'running' ? (status.syncing > 0 ? 'Syncing' : 'Up to date') : 'Paused' },
            { k: 'On this PC', v: onPc },
            { k: 'In vault', v: synced7d },
            { k: 'Last sync', v: lastSyncLabel },
          ].map(({ k, v }) => (
            <div key={k}>
              <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>
                {k}
              </div>
              <div style={{ fontSize: 15, fontWeight: 600, fontFamily: T.fontMono, color: T.ink }}>{v}</div>
            </div>
          ))}
        </div>
      </Card>

      <Card style={{ padding: 0, overflow: 'hidden' }}>
        {SYNC_ROWS.map((row, i) => {
          const isLast = i === SYNC_ROWS.length - 1
          let control: ReactNode = null

          if (row.locked) {
            control = <Toggle on={true} onChange={() => undefined} label={row.label} />
          } else if (row.label === 'Sync on metered connections') {
            control = <Toggle on={metered} onChange={(v) => onConfigChange({ metered: v })} label={row.label} />
          } else if (row.label === 'Files on Demand (online-only by default)') {
            control = <Toggle on={filesOnDemand} onChange={(v) => onConfigChange({ files_on_demand: v })} label={row.label} />
          } else if (row.label === 'Show sync overlays in File Explorer') {
            control = <Toggle on={overlays} onChange={(v) => onConfigChange({ sync_overlays: v })} label={row.label} />
          } else if (row.value !== undefined) {
            const limit = row.configKey === 'upload_kbps_limit' ? config.upload_kbps_limit : config.download_kbps_limit
            control = (
              <span style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>
                {limit === 0 ? 'Auto' : `${limit} KB/s`}
              </span>
            )
          }

          return (
            <div
              key={row.label}
              style={{ display: 'grid', gridTemplateColumns: '1fr 140px', padding: '14px 18px', alignItems: 'center', gap: 12, borderBottom: isLast ? 'none' : `1px solid ${T.line}` }}
            >
              <div>
                <div style={{ fontSize: 13, fontWeight: 500, color: T.ink, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' as const }}>
                  {row.label}
                  {row.locked && <Chip>Locked by design</Chip>}
                </div>
                <div style={{ fontSize: 11.5, color: T.ink3, marginTop: 3, lineHeight: 1.4 }}>{row.hint}</div>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', alignItems: 'center' }}>{control}</div>
            </div>
          )
        })}
      </Card>

      {notice && (
        <div style={{ marginTop: 14, padding: '10px 14px', borderRadius: 8, border: '1px solid oklch(0.88 0.05 84)', background: T.amberBg, fontSize: 12, color: T.amberDeep, lineHeight: 1.5 }}>
          {notice}
        </div>
      )}

      <Card style={{ marginTop: 24, padding: 18, display: 'flex', alignItems: 'center', gap: 14, background: T.paper2 }}>
        <div style={{ width: 32, height: 32, borderRadius: 8, background: T.paper, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <NavIcon name="cloud" size={14} color={T.amberDeep} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 500, color: T.ink, marginBottom: 2 }}>Free up space</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>
            Convert files you have not opened in 30 days to online-only. Keeps them in the vault, removes bytes from this PC.
          </div>
          {freeUpNote && <div style={{ fontSize: 11, color: T.ink3, marginTop: 4, fontFamily: T.fontMono }}>{freeUpNote}</div>}
        </div>
        <PrimaryBtn onClick={() => void freeUp()} disabled={freeUpBusy}>
          {freeUpBusy ? 'Working...' : 'Free up space'}
        </PrimaryBtn>
      </Card>
    </SettingsSectionShell>
  )
}

function NotificationPreferencesPanel() {
  const [values, setValues] = useState<NotificationPreferenceValues | null>(null)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [loading, setLoading] = useState(true)
  const [busyKey, setBusyKey] = useState<AccountNotificationPrefKey | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)

  const load = () => {
    let cancelled = false
    setLoading(true)
    setErr(null)
    setSaveError(null)
    void (async () => {
      const r = await accountNotificationPreferences()
      if (cancelled) return
      if (r.ok) {
        setValues(r.value.preferences)
        setErr(null)
      } else {
        setErr({ reason: r.reason, unsupported: r.unsupported })
        setValues(null)
      }
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => {
    let active = true
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountNotificationPreferences()
      if (!active) return
      if (r.ok) setValues(r.value.preferences)
      else setErr({ reason: r.reason, unsupported: r.unsupported })
      setLoading(false)
    })()
    return () => { active = false }
  }, [])

  const toggle = (key: AccountNotificationPrefKey) => {
    if (!values || busyKey) return
    const prev = values
    const next = !values[key]
    setValues({ ...values, [key]: next })
    setBusyKey(key)
    setSaveError(null)
    void (async () => {
      const r = await accountUpdateNotificationPreferences({ [key]: next })
      if (r.ok) {
        setValues(r.value.preferences)
      } else {
        setValues(prev)
        setSaveError(r.unsupported ? 'Changing this preference is not available in this build.' : `Could not save: ${r.reason}`)
      }
      setBusyKey(null)
    })()
  }

  if (loading) return <NotificationPreferencesSkeleton />
  if (err) return <ErrorBlock reason={err.reason} unsupported={err.unsupported} onRetry={load} />
  if (!values) {
    return (
      <Card style={{ padding: '30px 24px', display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' as const, gap: 8 }}>
        <NavIcon name="cog" size={16} color={T.ink4} />
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>No preferences to show</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6, maxWidth: 360 }}>Notification preferences could not be read for this account.</div>
      </Card>
    )
  }

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cog" size={14} color={T.ink2} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Notification preferences</div>
          <div style={{ fontSize: 11, color: T.ink3 }}>Choose which account events notify you.</div>
        </div>
      </div>

      {ACCOUNT_NOTIFICATION_PREF_META.map((m, i) => {
        const last = i === ACCOUNT_NOTIFICATION_PREF_META.length - 1
        const on = values[m.key]
        return (
          <div key={m.key} style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '14px 20px', alignItems: 'center', borderBottom: last ? 'none' : `1px solid ${T.line}` }}>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 500, color: T.ink }}>{m.label}</div>
              <div style={{ fontSize: 11, color: T.ink3, marginTop: 3, lineHeight: 1.5 }}>{m.hint}</div>
            </div>
            <Toggle on={on} busy={busyKey === m.key} onChange={() => toggle(m.key)} label={m.label} />
          </div>
        )
      })}

      {saveError && <div style={{ padding: '11px 20px', borderTop: `1px solid ${T.line}`, background: T.paper2, fontSize: 11.5, color: RED, lineHeight: 1.5 }}>{saveError}</div>}
    </Card>
  )
}

function NotificationPreferencesSkeleton() {
  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cog" size={14} color={T.ink4} />
        <Skeleton width={150} height={12} />
      </div>
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 36px', gap: 16, padding: '14px 20px', alignItems: 'center', borderBottom: i < 4 ? `1px solid ${T.line}` : 'none' }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <Skeleton width={`${44 - i * 4}%`} height={12} />
            <Skeleton width={`${68 - i * 4}%`} height={10} />
          </div>
          <Skeleton width={36} height={20} radius={999} />
        </div>
      ))}
    </Card>
  )
}

function DesktopNotificationPreferencesPanel({
  config,
  onConfigChange,
  notice,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  const toggleDesktopNotification = (key: DesktopNotificationPrefKey, value: boolean) => {
    switch (key) {
      case 'notify_conflicts':
        onConfigChange({ notify_conflicts: value })
        break
    }
  }

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="external" size={14} color={T.ink2} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Desktop alerts</div>
          <div style={{ fontSize: 11, color: T.ink3 }}>Native notifications fired by this device.</div>
        </div>
      </div>

      {DESKTOP_NOTIFICATION_PREF_META.map((m, i) => {
        const last = i === DESKTOP_NOTIFICATION_PREF_META.length - 1
        const on = config[m.key]
        return (
          <div key={m.key} style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '14px 20px', alignItems: 'center', borderBottom: last ? 'none' : `1px solid ${T.line}` }}>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 500, color: T.ink }}>{m.label}</div>
              <div style={{ fontSize: 11, color: T.ink3, marginTop: 3, lineHeight: 1.5 }}>{m.hint}</div>
            </div>
            <Toggle on={on} onChange={(value) => toggleDesktopNotification(m.key, value)} label={m.label} />
          </div>
        )
      })}

      {notice && <div style={{ padding: '11px 20px', borderTop: `1px solid ${T.line}`, background: T.paper2, fontSize: 11.5, color: RED, lineHeight: 1.5 }}>{notice}</div>}
    </Card>
  )
}

function NotificationsSettingsPanel({
  config,
  onConfigChange,
  notice,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  return (
    <SettingsSectionShell>
      <PageHeader
        title="Notifications"
        subtitle="Choose which desktop and account events Beebeeb is allowed to tell you about."
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        <DesktopNotificationPreferencesPanel config={config} onConfigChange={onConfigChange} notice={notice} />
        <NotificationPreferencesPanel />
      </div>
    </SettingsSectionShell>
  )
}

function LaunchPanel() {
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    command<boolean>('autostart_enabled').then((r) => {
      if (cancelled) return
      if (r.ok) setEnabled(r.value)
      else setError(r.unsupported ? commandUnavailableLabel('autostart_enabled') : r.reason)
    })
    return () => { cancelled = true }
  }, [])

  const toggle = async () => {
    setBusy(true)
    setError(null)
    const r = await command<boolean>('toggle_autostart')
    setBusy(false)
    if (r.ok) {
      setEnabled(r.value)
      return
    }
    setError(r.unsupported ? commandUnavailableLabel('toggle_autostart') : r.reason)
  }

  return (
    <SettingsSectionShell>
      <PageHeader title="Launch" subtitle="Control how Beebeeb starts on this PC." />
      {error && <div style={{ padding: '10px 12px', borderRadius: 6, marginBottom: 16, border: '1px solid oklch(0.88 0.05 25)', background: 'oklch(0.98 0.02 25)', color: RED, fontSize: 12 }}>{error}</div>}
      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Start at login</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>Beebeeb launches automatically and resumes syncing when you sign in to Windows.</div>
        </div>
        <Toggle on={enabled === true} busy={busy || enabled === null} onChange={() => void toggle()} label="Start at login" />
      </Card>
    </SettingsSectionShell>
  )
}

function ExplorerIntegrationPanel() {
  const [state, setState] = useState<ShellIntegrationState | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const regionCity = useRegionCity()

  const refresh = async () => {
    const r = await command<ShellIntegrationState>('windows_shell_integration_state')
    if (r.ok) {
      setState(r.value)
      setError(null)
    } else {
      setError(r.unsupported ? commandUnavailableLabel('windows_shell_integration_state') : r.reason)
    }
    setLoading(false)
  }

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const r = await command<ShellIntegrationState>('windows_shell_integration_state')
      if (cancelled) return
      if (r.ok) setState(r.value)
      else setError(r.unsupported ? commandUnavailableLabel('windows_shell_integration_state') : r.reason)
      setLoading(false)
    })()
    return () => { cancelled = true }
  }, [])

  const enable = async () => {
    setBusy(true)
    setError(null)
    const r = await command<ShellIntegrationState>('install_windows_shell_integration')
    setBusy(false)
    if (r.ok) {
      setState(r.value)
      void refresh()
      return
    }
    setError(r.unsupported ? commandUnavailableLabel('install_windows_shell_integration') : r.reason)
  }

  const active = state?.installed === true

  return (
    <SettingsSectionShell>
      <PageHeader
        title="Explorer integration"
        subtitle={`Beebeeb appears in File Explorer as a sync folder. Files are encrypted on this PC before they leave for ${regionCity}.`}
      />

      {error && <div style={{ padding: '10px 12px', borderRadius: 6, marginBottom: 16, border: '1px solid oklch(0.88 0.05 25)', background: 'oklch(0.98 0.02 25)', color: RED, fontSize: 12, lineHeight: 1.5 }}>{error}</div>}

      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>File Explorer location</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
            {loading ? 'Checking...' : active ? 'Active. Beebeeb is registered as a sync folder on this PC.' : 'Not set up yet on this PC.'}
          </div>
        </div>
        {!loading && !active && <PrimaryBtn onClick={() => void enable()} disabled={busy}>{busy ? 'Enabling...' : 'Enable'}</PrimaryBtn>}
        {!loading && active && <Chip tone="green">Active</Chip>}
      </Card>
    </SettingsSectionShell>
  )
}

function UpdatesPanel({
  config,
  onConfigChange,
  notice,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  const [version, setVersion] = useState<string | null>(null)
  const [updateCheckState, setUpdateCheckState] = useState<ManualUpdateCheckState>({ kind: 'idle' })
  const [downgradeInstallState, setDowngradeInstallState] = useState<'idle' | 'installing' | 'error'>('idle')
  const [downgradeInstallError, setDowngradeInstallError] = useState<string | null>(null)
  const updateCheckRequestRef = useRef(0)
  const releaseChannel = config.release_channel ?? 'stable'
  const updateCheckView = buildUpdateCheckViewModel(updateCheckState, version, releaseChannel)
  const updateCheckBusy = updateCheckState.kind === 'checking'
  const updateCheckError = updateCheckState.kind === 'error'
  const downgradeInstallBusy = downgradeInstallState === 'installing'

  useEffect(() => {
    let cancelled = false
    command<string>('app_version').then((r) => {
      if (!cancelled && r.ok) setVersion(r.value)
    })
    return () => { cancelled = true }
  }, [])

  const handleReleaseChannelChange = (channel: ReleaseChannelValue) => {
    if (channel === releaseChannel) return
    updateCheckRequestRef.current += 1
    setUpdateCheckState({ kind: 'idle' })
    setDowngradeInstallState('idle')
    setDowngradeInstallError(null)
    onConfigChange({ release_channel: channel })
  }

  const handleCheckForUpdates = async () => {
    const requestId = updateCheckRequestRef.current + 1
    updateCheckRequestRef.current = requestId
    setUpdateCheckState({ kind: 'checking' })
    setDowngradeInstallState('idle')
    setDowngradeInstallError(null)

    const result = await checkForDesktopUpdatesNow()
    if (updateCheckRequestRef.current !== requestId) return

    if (result.ok) {
      setUpdateCheckState(stateFromManualUpdateResult(result.value))
    } else {
      setUpdateCheckState({
        kind: 'error',
        reason: result.unsupported ? commandUnavailableLabel('check_for_updates_now') : result.reason,
      })
    }
  }

  const handleDowngrade = async () => {
    if (updateCheckState.kind !== 'downgrade_available') return

    const selectedChannel = releaseChannelLabel(updateCheckState.channel)
    const confirmed = window.confirm(
      `Downgrade Beebeeb from ${updateCheckState.currentVersion} to ${updateCheckState.version} on the ${selectedChannel} channel? The signed installer will run and Beebeeb will restart if it succeeds.`,
    )
    if (!confirmed) return

    setDowngradeInstallState('installing')
    setDowngradeInstallError(null)

    const result = await installDesktopChannelDowngrade(updateCheckState.version)
    if (result.ok) return

    setDowngradeInstallState('error')
    setDowngradeInstallError(result.reason)
  }

  return (
    <SettingsSectionShell>
      <PageHeader
        title="Updates"
        subtitle="Beebeeb checks for updates at launch and every 4 hours. When one is available, the banner appears at the top of the app."
      />

      <Card style={{ display: 'grid', gridTemplateColumns: 'auto minmax(0, 1fr) auto auto', alignItems: 'center', gap: 14, padding: '16px 18px', background: T.paper2, marginBottom: 16 }}>
        <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <NavIcon name="download" size={15} color={T.amberDeep} />
        </div>
        <div role={updateCheckError ? 'alert' : 'status'} aria-live="polite" style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>{updateCheckView.title}</div>
          <div style={{ fontSize: 11.5, color: updateCheckError ? RED : T.ink3, lineHeight: 1.4, fontFamily: T.fontMono, wordBreak: 'break-word' as const }}>
            {updateCheckView.detail}
          </div>
        </div>
        <Chip tone={updateCheckView.tone}>{updateCheckView.chip}</Chip>
        <button
          type="button"
          onClick={() => void handleCheckForUpdates()}
          disabled={updateCheckBusy || downgradeInstallBusy}
          aria-busy={updateCheckBusy}
          style={{
            minWidth: 142,
            height: 32,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 7,
            padding: '0 13px',
            fontSize: 12,
            fontFamily: T.fontSans,
            fontWeight: 600,
            borderRadius: 6,
            border: `1px solid ${T.amberDeep}`,
            background: updateCheckBusy ? T.amberBg : T.amber,
            color: T.ink,
            cursor: updateCheckBusy ? 'progress' : 'pointer',
            opacity: updateCheckBusy ? 0.72 : 1,
            whiteSpace: 'nowrap' as const,
            flexShrink: 0,
          }}
        >
          <NavIcon name="download" size={12} color={T.ink} />
          {updateCheckBusy ? 'Checking...' : 'Check for updates'}
        </button>
        {updateCheckState.kind === 'downgrade_available' && (
          <div style={{ gridColumn: '2 / -1', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, minWidth: 0 }}>
            <div
              role={downgradeInstallState === 'error' ? 'alert' : undefined}
              style={{
                minWidth: 0,
                fontSize: 11.5,
                color: downgradeInstallState === 'error' ? RED : T.ink3,
                lineHeight: 1.45,
                fontFamily: T.fontMono,
                wordBreak: 'break-word' as const,
              }}
            >
              {downgradeInstallState === 'error' && downgradeInstallError
                ? `Downgrade failed: ${downgradeInstallError}`
                : updateCheckState.body || 'The selected channel is behind this install. Downgrade only after confirming.'}
            </div>
            <div style={{ display: 'inline-flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
              <button
                type="button"
                onClick={() => void openUrl(updateCheckState.releaseNotesUrl)}
                disabled={downgradeInstallBusy}
                style={{
                  height: 30,
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 6,
                  padding: '0 11px',
                  fontSize: 11.5,
                  fontFamily: T.fontSans,
                  fontWeight: 600,
                  borderRadius: 6,
                  border: `1px solid ${T.line2}`,
                  background: T.paper,
                  color: T.ink2,
                  cursor: downgradeInstallBusy ? 'not-allowed' : 'pointer',
                  opacity: downgradeInstallBusy ? 0.6 : 1,
                  whiteSpace: 'nowrap' as const,
                }}
              >
                <NavIcon name="external" size={11} color={T.ink3} />
                Release notes
              </button>
              <button
                type="button"
                onClick={() => void handleDowngrade()}
                disabled={downgradeInstallBusy || updateCheckBusy}
                aria-busy={downgradeInstallBusy}
                style={{
                  height: 30,
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 6,
                  padding: '0 12px',
                  fontSize: 11.5,
                  fontFamily: T.fontSans,
                  fontWeight: 700,
                  borderRadius: 6,
                  border: `1px solid ${T.amberDeep}`,
                  background: downgradeInstallBusy ? T.amberBg : T.amber,
                  color: T.ink,
                  cursor: downgradeInstallBusy || updateCheckBusy ? 'progress' : 'pointer',
                  opacity: downgradeInstallBusy ? 0.72 : 1,
                  whiteSpace: 'nowrap' as const,
                }}
              >
                <NavIcon name="download" size={11} color={T.ink} />
                {downgradeInstallBusy ? 'Downgrading...' : `Downgrade to ${updateCheckState.version}`}
              </button>
            </div>
          </div>
        )}
      </Card>

      <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '15px 18px', alignItems: 'center', borderBottom: `1px solid ${T.line}` }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Release channel</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              Choose which desktop releases this PC receives.
            </div>
          </div>
          <div
            role="radiogroup"
            aria-label="Release channel"
            style={{
              display: 'inline-grid',
              gridTemplateColumns: 'repeat(3, minmax(66px, 1fr))',
              background: T.paper2,
              border: `1px solid ${T.line2}`,
              borderRadius: 7,
              padding: 3,
              gap: 3,
              flexShrink: 0,
            }}
          >
            {RELEASE_CHANNELS.map((channel) => {
              const selected = channel.value === releaseChannel
              return (
                <button
                  key={channel.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => handleReleaseChannelChange(channel.value)}
                  style={{
                    minWidth: 0,
                    height: 28,
                    padding: '0 10px',
                    borderRadius: 5,
                    border: selected ? `1px solid ${T.amberDeep}` : '1px solid transparent',
                    background: selected ? T.paper : 'transparent',
                    color: selected ? T.ink : T.ink2,
                    fontSize: 11.5,
                    fontWeight: selected ? 700 : 500,
                    fontFamily: T.fontSans,
                    cursor: 'pointer',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {channel.label}
                </button>
              )
            })}
          </div>
        </div>

        {RELEASE_CHANNELS.map((channel, index) => {
          const selected = channel.value === releaseChannel
          return (
            <div
              key={channel.value}
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr auto',
                gap: 12,
                padding: '11px 18px',
                alignItems: 'center',
                borderBottom: index === RELEASE_CHANNELS.length - 1 ? 'none' : `1px solid ${T.line}`,
              }}
            >
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 12.5, fontWeight: 500, color: T.ink }}>{channel.label}</div>
                <div style={{ fontSize: 11, color: T.ink3, marginTop: 2, lineHeight: 1.45 }}>{channel.hint}</div>
              </div>
              {selected && <Chip tone="green">Selected</Chip>}
            </div>
          )
        })}
      </Card>

      {notice && (
        <div style={{ margin: '0 0 16px', padding: '10px 14px', borderRadius: 8, border: '1px solid oklch(0.88 0.05 84)', background: T.amberBg, fontSize: 12, color: T.amberDeep, lineHeight: 1.5 }}>
          {notice}
        </div>
      )}

      <p style={{ margin: '0 0 16px', fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb checks for updates automatically at startup and every 4 hours.
      </p>
      <p style={{ margin: 0, fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Updates are verified with a minisign signature before installation. No update is applied without your confirmation via the restart banner.
      </p>
    </SettingsSectionShell>
  )
}

type AppActivityLoadState =
  | { phase: 'loading' }
  | { phase: 'error'; reason: string; unsupported: boolean }
  | { phase: 'ready'; snapshot: AppActivitySnapshot; updatedAt: number }

function ActivityMetric({
  label,
  value,
  detail,
  icon,
  accent = T.ink3,
}: {
  label: string
  value: string
  detail?: string
  icon: string
  accent?: string
}) {
  return (
    <div style={{ background: T.paper2, border: `1px solid ${T.line}`, borderRadius: 8, padding: '13px 14px', minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 8 }}>
        <NavIcon name={icon} size={12} color={accent} />
        <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.06em', color: T.ink4 }}>
          {label}
        </div>
      </div>
      <div style={{ fontSize: 22, lineHeight: 1.1, fontWeight: 700, fontFamily: T.fontMono, color: T.ink, overflowWrap: 'anywhere' as const }}>
        {value}
      </div>
      {detail && (
        <div style={{ marginTop: 5, fontSize: 10.5, lineHeight: 1.4, fontFamily: T.fontMono, color: T.ink4, overflowWrap: 'anywhere' as const }}>
          {detail}
        </div>
      )}
    </div>
  )
}

function AppActivityPanel() {
  const [state, setState] = useState<AppActivityLoadState>({ phase: 'loading' })
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false

    const load = async () => {
      const result = await appActivitySnapshot()
      if (cancelled) return
      if (result.ok) {
        setState({ phase: 'ready', snapshot: result.value, updatedAt: Date.now() })
      } else {
        setState((prev) => (
          prev.phase === 'ready'
            ? prev
            : { phase: 'error', reason: result.reason, unsupported: result.unsupported }
        ))
      }
    }

    void load()
    const id = window.setInterval(() => { void load() }, ACTIVITY_POLL_INTERVAL_MS)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [reloadKey])

  if (state.phase === 'loading') {
    return (
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(145px, 1fr))', gap: 12 }}>
          {Array.from({ length: 4 }).map((_, index) => (
            <Skeleton key={index} height={84} radius={8} />
          ))}
        </div>
      </Card>
    )
  }

  if (state.phase === 'error') {
    return (
      <ErrorBlock
        reason={state.reason}
        unsupported={state.unsupported}
        onRetry={() => {
          setState({ phase: 'loading' })
          setReloadKey((key) => key + 1)
        }}
      />
    )
  }

  const { snapshot } = state
  const sampleAge = formatSampleAge(snapshot.throughput_sample_age_ms)
  const period = snapshot.throughput_period_secs ?? null
  const networkDetail = period != null && snapshot.throughput_sample_age_ms != null
    ? `heartbeat ${sampleAge}, ${period}s window`
    : sampleAge

  return (
    <Card style={{ padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 14, marginBottom: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
          <div style={{ width: 32, height: 32, borderRadius: 8, background: T.amberBg, border: '1px solid oklch(0.86 0.07 90)', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="chart" size={15} color={T.amberDeep} />
          </div>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>App Activity</div>
            <div style={{ fontSize: 11, color: T.ink3, fontFamily: T.fontMono, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
              {snapshot.process_name || 'Beebeeb'} · PID {snapshot.pid}
            </div>
          </div>
        </div>
        <Chip tone="neutral">Updated {formatSampleAge(Date.now() - state.updatedAt)}</Chip>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(145px, 1fr))', gap: 12 }}>
        <ActivityMetric
          label="CPU"
          value={formatCpuPercent(snapshot.cpu_percent)}
          detail="current process"
          icon="bolt"
          accent={T.amberDeep}
        />
        <ActivityMetric
          label="Memory RSS"
          value={formatBytes(snapshot.memory_rss_bytes)}
          detail="resident set"
          icon="device"
        />
        <ActivityMetric
          label="Upload"
          value={formatRate(snapshot.upload_bps)}
          detail={networkDetail}
          icon="cloud"
          accent={T.amberDeep}
        />
        <ActivityMetric
          label="Download"
          value={formatRate(snapshot.download_bps)}
          detail={networkDetail}
          icon="download"
        />
      </div>
    </Card>
  )
}

function AdvancedPanel({
  storage,
  config,
  onConfigChange,
  notice,
}: {
  storage: StorageSummary | null
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  const themePreference = normalizeThemePreference(config.theme)
  const cacheLimitBytes = normalizeCacheLimitBytes(config.local_cache_limit_bytes)
  const cacheView = buildCacheLimitView({
    cacheBytes: storage?.cache_bytes ?? 0,
    pinnedBytes: storage?.pinned_bytes ?? 0,
    limitBytes: cacheLimitBytes,
  })
  const usageKnown = storage != null

  const chooseTheme = (theme: DesktopTheme) => {
    if (theme === themePreference) return
    setDesktopThemePreference(theme)
    onConfigChange({ theme })
  }

  const chooseCacheLimit = (value: number) => {
    if (value === cacheLimitBytes) return
    onConfigChange({ local_cache_limit_bytes: value })
  }

  return (
    <SettingsSectionShell>
      <PageHeader
        title="Advanced"
        subtitle="Device-level behavior for this desktop app."
      />

      <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '15px 18px', alignItems: 'center', borderBottom: `1px solid ${T.line}` }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Theme</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              Choose the appearance for this app window.
            </div>
          </div>
          <div
            role="radiogroup"
            aria-label="Theme"
            style={{
              display: 'inline-grid',
              gridTemplateColumns: 'repeat(3, minmax(72px, 1fr))',
              background: T.paper2,
              border: `1px solid ${T.line2}`,
              borderRadius: 7,
              padding: 3,
              gap: 3,
              flexShrink: 0,
            }}
          >
            {THEME_OPTIONS.map((option) => {
              const selected = option.value === themePreference
              return (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => chooseTheme(option.value)}
                  style={{
                    minWidth: 0,
                    height: 28,
                    padding: '0 10px',
                    borderRadius: 5,
                    border: selected ? `1px solid ${T.amberDeep}` : '1px solid transparent',
                    background: selected ? T.paper : 'transparent',
                    color: selected ? T.ink : T.ink2,
                    fontSize: 11.5,
                    fontWeight: selected ? 700 : 500,
                    fontFamily: T.fontSans,
                    cursor: 'pointer',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {option.label}
                </button>
              )
            })}
          </div>
        </div>
        {THEME_OPTIONS.map((option, index) => {
          const selected = option.value === themePreference
          return (
            <div
              key={option.value}
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr auto',
                gap: 12,
                padding: '11px 18px',
                alignItems: 'center',
                borderBottom: index === THEME_OPTIONS.length - 1 ? 'none' : `1px solid ${T.line}`,
              }}
            >
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 12.5, fontWeight: 500, color: T.ink }}>{option.label}</div>
                <div style={{ fontSize: 11, color: T.ink3, marginTop: 2, lineHeight: 1.45 }}>{option.hint}</div>
              </div>
              {selected && <Chip tone="green">Selected</Chip>}
            </div>
          )
        })}
      </Card>

      <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '15px 18px', alignItems: 'center', borderBottom: `1px solid ${T.line}` }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Local cache limit</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              Cap file content stored on this device.
            </div>
          </div>
          <div
            role="radiogroup"
            aria-label="Local cache limit"
            style={{
              display: 'inline-grid',
              gridTemplateColumns: 'repeat(4, minmax(76px, 1fr))',
              background: T.paper2,
              border: `1px solid ${T.line2}`,
              borderRadius: 7,
              padding: 3,
              gap: 3,
              flexShrink: 0,
            }}
          >
            {CACHE_LIMIT_OPTIONS.map((option) => {
              const selected = option.value === cacheLimitBytes
              return (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => chooseCacheLimit(option.value)}
                  style={{
                    minWidth: 0,
                    height: 28,
                    padding: '0 10px',
                    borderRadius: 5,
                    border: selected ? `1px solid ${T.amberDeep}` : '1px solid transparent',
                    background: selected ? T.paper : 'transparent',
                    color: selected ? T.ink : T.ink2,
                    fontSize: 11.5,
                    fontWeight: selected ? 700 : 500,
                    fontFamily: T.fontSans,
                    cursor: 'pointer',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {option.label}
                </button>
              )
            })}
          </div>
        </div>

        <div style={{ padding: '15px 18px' }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 12, alignItems: 'center', marginBottom: 9 }}>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>
                {usageKnown ? cacheView.usageLabel : 'Usage unavailable'}
              </div>
              <div style={{ fontSize: 11, color: T.ink3, marginTop: 3, lineHeight: 1.45 }}>
                {usageKnown
                  ? `Unpinned cache ${formatBytes(storage.cache_bytes)} · pinned ${formatBytes(storage.pinned_bytes)}`
                  : 'Start the sync engine to calculate local file content.'}
              </div>
            </div>
            <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, whiteSpace: 'nowrap' }}>
              Cap {cacheView.capLabel}
            </div>
          </div>
          <div style={{ height: 8, borderRadius: 999, background: T.paper3, overflow: 'hidden', border: `1px solid ${T.line}` }}>
            <div
              style={{
                width: `${cacheView.percentUsed ?? 0}%`,
                height: '100%',
                borderRadius: 999,
                background: cacheView.pinnedExceedsCap ? RED : T.amberDeep,
                transition: 'width 160ms ease',
              }}
            />
          </div>
        </div>

        {cacheView.warning && (
          <div role="alert" style={{ padding: '11px 18px', borderTop: `1px solid ${T.line}`, background: T.amberBg, color: T.amberDeep, fontSize: 11.5, lineHeight: 1.5 }}>
            {cacheView.warning}
          </div>
        )}
      </Card>

      {notice && (
        <div style={{ margin: '0 0 16px', padding: '10px 14px', borderRadius: 8, border: '1px solid oklch(0.88 0.05 84)', background: T.amberBg, fontSize: 12, color: T.amberDeep, lineHeight: 1.5 }}>
          {notice}
        </div>
      )}

      <AppActivityPanel />
    </SettingsSectionShell>
  )
}

function SettingsNav({
  activeNav,
  loggedIn,
  onChange,
}: {
  activeNav: SettingsNavId
  loggedIn: boolean
  onChange: (id: SettingsNavId) => void
}) {
  const sections = availableSettingsSections(loggedIn)

  return (
    <div style={{ background: T.paper2, borderRight: `1px solid ${T.line}`, padding: '16px 10px', overflow: 'auto', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
      <div style={{ padding: '4px 8px 10px' }}>
        <div style={{ fontSize: 13, fontWeight: 700, color: T.ink, marginBottom: 3 }}>Settings</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>Device, sync, updates, and notifications.</div>
      </div>

      {sections.map((section) => (
        <div key={section.heading} style={{ marginBottom: 10 }}>
          <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, padding: '4px 10px', marginBottom: 2 }}>
            {section.heading}
          </div>
          {section.items.map((item) => {
            const active = item.id === activeNav
            return (
              <button
                key={item.id}
                onClick={() => onChange(item.id)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  width: '100%',
                  padding: '7px 10px',
                  borderRadius: 6,
                  fontSize: 12.5,
                  fontWeight: active ? 600 : 400,
                  color: active ? T.ink : T.ink2,
                  background: active ? T.paper : 'transparent',
                  border: active ? `1px solid ${T.line}` : '1px solid transparent',
                  borderLeft: active ? `3px solid ${T.amberDeep}` : '3px solid transparent',
                  cursor: 'pointer',
                  textAlign: 'left' as const,
                  fontFamily: T.fontSans,
                  marginBottom: 2,
                  transition: 'background 100ms',
                }}
              >
                <NavIcon name={item.icon} size={12} color={active ? T.amberDeep : T.ink3} />
                {item.label}
              </button>
            )
          })}
        </div>
      ))}
    </div>
  )
}

export default function SettingsView({ status, onOpenSignIn }: SettingsViewProps) {
  const loggedIn = status?.logged_in ?? false
  const [activeNav, setActiveNav] = useState<SettingsNavId>(() => defaultSettingsPage(loggedIn))
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [config, setConfig] = useState<DesktopConfig>(DEFAULT_CONFIG)
  const configRef = useRef<DesktopConfig>(DEFAULT_CONFIG)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    if (!loggedIn && !ALWAYS_ACCESSIBLE_SETTINGS.has(activeNav)) {
      setActiveNav(defaultSettingsPage(false))
    }
  }, [loggedIn, activeNav])

  useEffect(() => {
    if (!status?.logged_in || status.engine !== 'running') return
    let cancelled = false
    command<StorageSummary>('desktop_storage_summary').then((result) => {
      if (cancelled) return
      if (result.ok) setStorage(result.value)
    })
    return () => { cancelled = true }
  }, [status?.logged_in, status?.engine])

  useEffect(() => {
    command<DesktopConfig | null>('desktop_config').then((result) => {
      if (result.ok && result.value != null) {
        const loaded = { ...DEFAULT_CONFIG, ...result.value }
        configRef.current = loaded
        setConfig(loaded)
        setDesktopThemePreference(loaded.theme)
      }
    })
  }, [])

  const handleConfigChange = async (patch: Partial<DesktopConfig>) => {
    setNotice(null)
    if (patch.theme) setDesktopThemePreference(patch.theme)
    const next: DesktopConfig = { ...configRef.current, ...patch }
    configRef.current = next
    setConfig(next)
    const result = await command<void>('set_desktop_config', { config: next })
    if (!result.ok) {
      setNotice(result.reason)
    }
  }

  const renderPanel = () => {
    if (!loggedIn && !ALWAYS_ACCESSIBLE_SETTINGS.has(activeNav)) {
      return <SignedOutSettingsGate onOpenSignIn={onOpenSignIn} />
    }

    switch (activeNav) {
      case 'sync':
        return (
          <SyncPanel
            status={status}
            storage={storage}
            config={config}
            onConfigChange={(patch) => void handleConfigChange(patch)}
            notice={notice}
          />
        )
      case 'notifications':
        return (
          <NotificationsSettingsPanel
            config={config}
            onConfigChange={(patch) => void handleConfigChange(patch)}
            notice={notice}
          />
        )
      case 'launch':
        return <LaunchPanel />
      case 'explorer-integration':
        return <ExplorerIntegrationPanel />
      case 'updates':
        return <UpdatesPanel config={config} onConfigChange={(patch) => void handleConfigChange(patch)} notice={notice} />
      case 'advanced':
        return (
          <AdvancedPanel
            storage={storage}
            config={config}
            onConfigChange={(patch) => void handleConfigChange(patch)}
            notice={notice}
          />
        )
      default:
        return (
          <SettingsSectionShell>
            <PageHeader title={settingLabel(activeNav)} />
          </SettingsSectionShell>
        )
    }
  }

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '220px minmax(0, 1fr)', flex: 1, overflow: 'hidden', minHeight: 0 }}>
      <SettingsNav
        activeNav={activeNav}
        loggedIn={loggedIn}
        onChange={setActiveNav}
      />
      {renderPanel()}
    </div>
  )
}
