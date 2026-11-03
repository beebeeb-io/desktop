/**
 * The Sync panel: engine state, pause control, check progress and the per-folder rows.
 * It reads live status from the caller and reports config changes upward; it never writes
 * config itself.
 */
import { useCapabilities } from '../../../capabilities'
import { CapabilityNotice } from '../../../CapabilityNotice'
import { useState, type ReactNode } from 'react'
import { command, commandUnavailableLabel, formatBytes, type DesktopConfig, type StorageSummary, type SyncStatus } from '../../../desktopApi'
import { usePlatformName, thisDeviceNoun } from '../../../platform'
import { T, Card, Chip, NavIcon, PageHeader, PrimaryBtn, useToast } from '../../ui'
import { syncActivityView, type SyncActivityView } from '../../syncActivityModel'
import { SyncActivityIcon, SyncCheckProgress, SyncPauseButton, useSyncActivitySummary, useSyncPauseToggle } from '../../syncActivityUi'
import { FoundWhilePausedCard } from '../../FoundWhilePausedCard'
import { Toggle, SettingsSectionShell } from './primitives'

export interface SyncRow {
  label: string
  hint: string
  locked?: boolean
  value?: string
  configKey?: keyof DesktopConfig
}

export const SYNC_ROWS: SyncRow[] = [
  {
    label: 'End-to-end encryption',
    hint: 'Files are encrypted before upload. This cannot be disabled.',
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

export function SyncPanel({
  status,
  storage,
  config,
  onConfigChange,
}: {
  status: SyncStatus | null
  storage: StorageSummary | null
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
}) {
  const caps = useCapabilities()
  const windows = caps?.host_os === 'windows'
  const rows = SYNC_ROWS.filter((row) => !windows || (
    (row.label !== 'Sync on metered connections' || caps.metered_sync_control) &&
    (row.label !== 'Show sync overlays in File Explorer' || caps.sync_overlay_control) &&
    (row.label !== 'Files on Demand (online-only by default)' || caps.hydrate_all_control)
  ))
  const [freeUpBusy, setFreeUpBusy] = useState(false)
  // Not an error surface: freeUpNote reports the RESULT of 'Free up space' on both success and
  // failure, in ink3, not as an error. It happens to carry result.reason on the failure branch.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [freeUpNote, setFreeUpNote] = useState<string | null>(null)
  const { showToast } = useToast()
  const platform = usePlatformName()
  // What sync is doing (0.8.15): the live pause flag, the queue in counts and the startup check, next to `status`.
  // Before this the card read only `status`, so 2,924 changes on hold looked like "Up to date".
  const { summary, refresh } = useSyncActivitySummary()
  const view = syncActivityView(status, summary, { device: thisDeviceNoun(platform), pausedFallback: config.pause_sync })
  const pause = useSyncPauseToggle(view, refresh, (failure) => showToast({ variant: 'error', ...failure }))
  const lastSyncLabel = lastCheckLabel(summary?.last_tick_ok_at ?? null, summary === undefined)

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
        aside={<Chip tone={view.tone === 'good' ? 'green' : 'neutral'}>{view.label}</Chip>}
      />

      <Card style={{ padding: 0, marginBottom: 22, overflow: 'hidden' }}>
        <SyncStateHeader view={view} busy={pause.busy} onToggle={() => void pause.toggle()} />
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', gap: 20, padding: '14px 20px 16px', borderTop: `1px solid ${T.line}`, background: T.paper2 }}>
          {[
            { k: 'State', v: view.label },
            { k: `On ${thisDeviceNoun(platform)}`, v: onPc },
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

      {/* Task 2267: changes found while sync was paused wait here for the person's decision (draws nothing otherwise). */}
      <FoundWhilePausedCard summary={summary} refresh={refresh} />

      <Card style={{ padding: 0, overflow: 'hidden' }}>
        {rows.map((row, i) => {
          const isLast = i === rows.length - 1
          let control: ReactNode = null

          if (row.locked) {
            control = <Chip>Always on</Chip>
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
                <div style={{ fontSize: 11.5, color: T.ink3, marginTop: 3, lineHeight: 1.4 }}>{windows && row.locked ? 'Files are encrypted before upload. Downloaded files can be opened on this PC.' : row.hint}</div>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', alignItems: 'center' }}>{control}</div>
            </div>
          )
        })}
      </Card>

      {windows && <CapabilityNotice style={{ marginTop: 24 }}>Files download when opened. Use Selective sync to make folders online-only.
        Automatic metered-network control and overlay preferences are not available yet.</CapabilityNotice>}
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

/**
 * "Last sync" from the engine's last check that reached the server since the app started. Never a guess: no check yet
 * is "Not yet", and an unread summary is "…".
 */
export function lastCheckLabel(at: number | null, reading: boolean, now: number = Date.now()): string {
  if (reading) return '…'
  if (at == null || at <= 0) return 'Not yet'
  const sec = Math.max(0, Math.round(now / 1000 - at))
  if (sec < 45) return 'Just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr} hr ago`
  const day = Math.round(hr / 24)
  return `${day} day${day === 1 ? '' : 's'} ago`
}

/** The live state, in words, with Pause / Resume beside it (0.8.15; before it the only way to pause was the menu). */
export function SyncStateHeader({ view, busy, onToggle }: { view: SyncActivityView; busy: boolean; onToggle: () => void }) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14, padding: '18px 20px' }}>
      <div style={{ paddingTop: 1 }}>
        <SyncActivityIcon view={view} size={20} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }} aria-live="polite">
        <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, lineHeight: 1.35 }}>{view.headline}</div>
        {view.details.map((line) => (
          <div key={line} style={{ fontSize: 12, color: T.ink2, marginTop: 4, lineHeight: 1.45 }}>{line}</div>
        ))}
        {view.progress && <SyncCheckProgress check={view.progress} />}
      </div>
      <SyncPauseButton view={view} busy={busy} onToggle={onToggle} />
    </div>
  )
}
