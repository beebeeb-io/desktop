/**
 * Appearance and storage: theme choice, cache limit and the free-space action. It hosts the
 * app-activity readings so the two diagnostics views stay on one page.
 */
import { useCapabilities } from '../../../capabilities'
import { CapabilityNotice } from '../../../CapabilityNotice'
import { formatBytes, type DesktopConfig, type DesktopTheme, type StorageSummary } from '../../../desktopApi'
import { usePlatformName, type PlatformName } from '../../../platform'
import { T, Card, PageHeader } from '../../ui'
import { CACHE_LIMIT_OPTIONS, buildCacheLimitView, normalizeCacheLimitBytes, normalizeThemePreference } from '../../advancedSettingsModel'
import { setDesktopThemePreference } from '../../theme'
import { RED, SettingsSectionShell } from './primitives'
import { AppActivityPanel } from './appActivityPanel'

export const THEME_OPTIONS: Array<{ value: DesktopTheme; label: string; hint: string }> = [
  { value: 'light', label: 'Light', hint: 'Use the light desktop palette.' },
  { value: 'dark', label: 'Dark', hint: 'Use the dark desktop palette.' },
  { value: 'system', label: 'System', hint: 'Follow Windows or macOS.' },
]

export function systemThemeHint(platform: PlatformName): string {
  if (platform === 'macos') return 'Follow macOS.'
  if (platform === 'linux') return 'Follow your desktop.'
  return 'Follow Windows.'
}

export function AdvancedPanel({
  storage,
  config,
  onConfigChange,
}: {
  storage: StorageSummary | null
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
}) {
  const caps = useCapabilities()
  const cacheControl = caps?.host_os !== 'windows' || caps.cache_limit_control
  const platform = usePlatformName()
  const themeOptions = THEME_OPTIONS.map((option) =>
    option.value === 'system' ? { ...option, hint: systemThemeHint(platform) } : option,
  )
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
        title="Appearance & storage"
        subtitle="Theme and downloaded files on this device."
      />

      <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '15px 18px', alignItems: 'center', borderBottom: `1px solid ${T.line}` }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Theme</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              {themeOptions.find((option) => option.value === themePreference)?.hint}
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
            {themeOptions.map((option) => {
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
      </Card>

      {cacheControl ? <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
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
      </Card> : <CapabilityNotice>Automatic disk cache limits are not available on Windows yet. Use Free up space in Sync settings to reclaim downloaded files.</CapabilityNotice>}

      <AppActivityPanel />
    </SettingsSectionShell>
  )
}
