/**
 * Notification preferences: account-wide events and this-device switches, each written
 * through the caller so only the touched fields travel to the backend.
 */
import { useEffect, useState } from 'react'
import { accountNotificationPreferences, accountUpdateNotificationPreferences, type DesktopConfig, type NotificationPreferenceValues } from '../../../desktopApi'
import { ACCOUNT_NOTIFICATION_PREF_META, DESKTOP_NOTIFICATION_PREF_META, type AccountNotificationPrefKey, type DesktopNotificationPrefKey } from '../../notificationPreferenceMeta'
import { T, Card, NavIcon, PageHeader, Skeleton, useToast } from '../../ui'
import { Toggle, ErrorBlock, SettingsSectionShell } from './primitives'

export function NotificationPreferencesPanel() {
  const [values, setValues] = useState<NotificationPreferenceValues | null>(null)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [loading, setLoading] = useState(true)
  const [busyKey, setBusyKey] = useState<AccountNotificationPrefKey | null>(null)
  const { showToast } = useToast()

  const load = () => {
    let cancelled = false
    setLoading(true)
    setErr(null)
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
    void (async () => {
      const r = await accountUpdateNotificationPreferences({ [key]: next })
      if (r.ok) {
        setValues(r.value.preferences)
      } else {
        // Transient ACTION failure (a one-shot toggle press) -> Toast, matching
        // LaunchPanel's autostart toggle one panel over. The panel's OTHER error
        // (`err`, the load failure) deliberately stays inline via ErrorBlock+Retry.
        setValues(prev)
        showToast({
          variant: 'error',
          title: 'Couldn’t save notification preference',
          message: r.unsupported ? 'Changing this preference is not available in this build.' : r.reason,
        })
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
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Account notifications</div>
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
    </Card>
  )
}

export function NotificationPreferencesSkeleton() {
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

export function DesktopNotificationPreferencesPanel({
  config,
  onConfigChange,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
}) {
  const toggleDesktopNotification = (key: DesktopNotificationPrefKey, value: boolean) => {
    switch (key) {
      case 'notify_conflicts':
        onConfigChange({ notify_conflicts: value })
        break
      case 'notify_sync_complete':
        onConfigChange({ notify_sync_complete: value })
        break
      case 'notify_quota_warnings':
        onConfigChange({ notify_quota_warnings: value })
        break
    }
  }

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="external" size={14} color={T.ink2} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>On this device</div>
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

    </Card>
  )
}

export function NotificationsSettingsPanel({
  config,
  onConfigChange,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
}) {
  return (
    <SettingsSectionShell>
      <PageHeader
        title="Notifications"
        subtitle="Choose which desktop and account events Beebeeb is allowed to tell you about."
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        <DesktopNotificationPreferencesPanel config={config} onConfigChange={onConfigChange} />
        <NotificationPreferencesPanel />
      </div>
    </SettingsSectionShell>
  )
}
