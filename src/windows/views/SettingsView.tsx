/**
 * The Settings window: the navigation rail plus the panel the selected route names.
 * Each panel lives in its own module under `SettingsView/`; this file owns the route switch,
 * the config read/write and the theme side effect, and re-exports the panels other surfaces
 * import by name.
 */
export { DataResidencyContent } from './SettingsView/dataResidencyPanel'
export { UpdatesPanel } from './SettingsView/updatesPanel'
export { AdvancedPanel } from './SettingsView/advancedPanel'

import { useCapabilities, supportsRoute, CapabilityAlternative } from '../../capabilities'
import { useEffect, useRef, useState } from 'react'
import { command, type DesktopConfig, type StorageSummary, type SyncStatus, DEFAULT_CONFIG } from '../../desktopApi'
import { ALWAYS_ACCESSIBLE_SETTINGS, defaultSettingsPage, settingLabel, type SettingsNavId } from '../settingsNavigation'
import { usePlatform } from '../../platform'
import { PageHeader, useToast } from '../ui'
import { setDesktopThemePreference } from '../theme'
import { SignedOutSettingsGate, SettingsSectionShell } from './SettingsView/primitives'
import { SyncPanel } from './SettingsView/syncPanel'
import { NotificationsSettingsPanel } from './SettingsView/notificationsPanel'
import { DataResidencyPanel } from './SettingsView/dataResidencyPanel'
import { LaunchPanel } from './SettingsView/launchPanel'
import { ShellOrFinderPanel } from './SettingsView/integrationPanels'
import { UpdatesPanel } from './SettingsView/updatesPanel'
import { AdvancedPanel } from './SettingsView/advancedPanel'
import { SettingsNav } from './SettingsView/settingsNav'

interface SettingsViewProps {
  status: SyncStatus | null
  onOpenSignIn: () => void
}

export default function SettingsView({ status, onOpenSignIn }: SettingsViewProps) {
  const caps = useCapabilities()
  const { name: platform, resolved: platformResolved } = usePlatform()
  const loggedIn = status?.logged_in ?? false
  const [activeNav, setActiveNav] = useState<SettingsNavId>(() => defaultSettingsPage(loggedIn))
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [config, setConfig] = useState<DesktopConfig>(DEFAULT_CONFIG)
  const configRef = useRef<DesktopConfig>(DEFAULT_CONFIG)
  const { showToast } = useToast()

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
    if (patch.theme) setDesktopThemePreference(patch.theme)
    const next: DesktopConfig = { ...configRef.current, ...patch }
    configRef.current = next
    setConfig(next)
    // Only the fields this change touched are written (task 1889, fix round 1 M5): the object this window read on
    // open is never sent back, so a pause made from the tray meanwhile is not undone.
    const result = await command<void>('set_desktop_config', { config: patch })
    if (!result.ok) {
      // Transient ACTION failure (a settings control the user just changed) -> Toast.
      // This used to be a `notice` string prop-drilled into four panels, which rendered
      // it in two different visual treatments (amber in three, raw color: RED in one).
      showToast({ variant: 'error', title: 'Couldn’t save setting', message: result.reason })
    }
  }

  const renderPanel = () => {
    if (!supportsRoute(caps, activeNav)) return <CapabilityAlternative />
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
          />
        )
      case 'notifications':
        return (
          <NotificationsSettingsPanel
            config={config}
            onConfigChange={(patch) => void handleConfigChange(patch)}
          />
        )
      case 'data-residency':
        return <DataResidencyPanel />
      case 'launch':
        return <LaunchPanel />
      case 'explorer-integration':
        return <ShellOrFinderPanel status={status} />
      case 'updates':
        return <UpdatesPanel config={config} onConfigChange={(patch) => void handleConfigChange(patch)} />
      case 'advanced':
        return (
          <AdvancedPanel
            storage={storage}
            config={config}
            onConfigChange={(patch) => void handleConfigChange(patch)}
          />
        )
      default:
        return (
          <SettingsSectionShell>
            <PageHeader title={settingLabel(activeNav, platformResolved ? platform : null)} />
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
