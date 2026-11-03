/**
 * Startup behaviour: sign in to the OS account on launch, and nothing else.
 */
import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel } from '../../../desktopApi'
import { usePlatformName, thisDeviceNoun, type PlatformName } from '../../../platform'
import { T, Card, PageHeader, useToast } from '../../ui'
import { Toggle, SettingsSectionShell } from './primitives'

export function signInToOsLine(platform: PlatformName): string {
  if (platform === 'macos') return 'Beebeeb launches automatically and resumes syncing when you sign in to macOS.'
  if (platform === 'linux') return 'Beebeeb launches automatically and resumes syncing when you sign in.'
  return 'Beebeeb launches automatically and resumes syncing when you sign in to Windows.'
}

export function LaunchPanel() {
  const platform = usePlatformName()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const { showToast } = useToast()

  useEffect(() => {
    let cancelled = false
    command<boolean>('autostart_enabled').then((r) => {
      if (cancelled) return
      if (r.ok) setEnabled(r.value)
      else showToast({
        variant: 'error',
        title: 'Couldn’t read launch setting',
        message: r.unsupported ? commandUnavailableLabel('autostart_enabled') : r.reason,
      })
    })
    return () => { cancelled = true }
  }, [showToast])

  const toggle = async () => {
    setBusy(true)
    const r = await command<boolean>('toggle_autostart')
    setBusy(false)
    if (r.ok) {
      setEnabled(r.value)
      return
    }
    showToast({
      variant: 'error',
      title: 'Couldn’t change launch setting',
      message: r.unsupported ? commandUnavailableLabel('toggle_autostart') : r.reason,
    })
  }

  return (
    <SettingsSectionShell>
      <PageHeader title="Launch" subtitle={`Control how Beebeeb starts on ${thisDeviceNoun(platform)}.`} />
      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Start at login</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>{signInToOsLine(platform)}</div>
        </div>
        <Toggle on={enabled === true} busy={busy || enabled === null} onChange={() => void toggle()} label="Start at login" />
      </Card>
    </SettingsSectionShell>
  )
}
