/**
 * The Updates panel: installed version, release channel, manual check and the downgrade
 * confirmation flow. The channel choice is the only setting it writes.
 */
import { useCapabilities, canInstallUpdate } from '../../../capabilities'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { appVersionFields } from '../../../appVersion'
import { command, installDesktopChannelDowngrade, openUrl, type DesktopConfig } from '../../../desktopApi'
import { RELEASE_CHANNELS } from '../../../settingsConstants'
import { usePlatformName, thisDeviceNoun } from '../../../platform'
import { buildDowngradeConfirmationViewModel, buildUpdateCheckViewModel } from '../../updateCheckViewModel'
import { T, Card, Chip, Modal, NavIcon, PageHeader } from '../../ui'
import { desktopUpdateCheck } from '../../manualUpdateCheck'
import { RED, SettingsSectionShell } from './primitives'

export type ReleaseChannelValue = NonNullable<DesktopConfig['release_channel']>

export function UpdatesPanel({
  config,
  onConfigChange,
}: {
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
}) {
  const caps = useCapabilities()
  const installSupported = canInstallUpdate(caps)
  const platform = usePlatformName()
  const [version, setVersion] = useState<string | null>(null)
  const updateCheckState = useSyncExternalStore(desktopUpdateCheck.subscribe, desktopUpdateCheck.getSnapshot, desktopUpdateCheck.getSnapshot)
  const [downgradeInstallState, setDowngradeInstallState] = useState<'idle' | 'installing' | 'error'>('idle')
  const [downgradeInstallError, setDowngradeInstallError] = useState<string | null>(null)
  const [downgradeConfirmOpen, setDowngradeConfirmOpen] = useState(false)
  const [releaseNotesOpen, setReleaseNotesOpen] = useState(false)
  const downgradeConfirmButtonRef = useRef<HTMLButtonElement | null>(null)
  const releaseChannel = config.release_channel ?? 'stable'
  const updateCheckView = buildUpdateCheckViewModel(updateCheckState, version, releaseChannel)
  const downgradeConfirmationView =
    updateCheckState.kind === 'downgrade_available'
      ? buildDowngradeConfirmationViewModel(updateCheckState)
      : null
  const updateCheckBusy = updateCheckState.kind === 'checking'
  const updateCheckError = updateCheckState.kind === 'error'
  const downgradeInstallBusy = downgradeInstallState === 'installing'

  useEffect(() => {
    let cancelled = false
    command<unknown>('app_version').then((r) => {
      if (!cancelled && r.ok) setVersion(appVersionFields(r.value)?.version ?? null)
    })
    return () => { cancelled = true }
  }, [])

  const handleReleaseChannelChange = (channel: ReleaseChannelValue) => {
    if (channel === releaseChannel) return
    desktopUpdateCheck.reset()
    setDowngradeInstallState('idle')
    setDowngradeInstallError(null)
    setDowngradeConfirmOpen(false)
    setReleaseNotesOpen(false)
    onConfigChange({ release_channel: channel })
  }

  useEffect(() => {
    if (updateCheckState.kind !== 'checking') return
    setDowngradeInstallState('idle')
    setDowngradeInstallError(null)
    setDowngradeConfirmOpen(false)
    setReleaseNotesOpen(false)
  }, [updateCheckState])

  const openDowngradeConfirmation = () => {
    if (!installSupported || updateCheckState.kind !== 'downgrade_available') return
    setReleaseNotesOpen(false)
    setDowngradeConfirmOpen(true)
  }

  const handleConfirmDowngrade = async () => {
    if (!installSupported || updateCheckState.kind !== 'downgrade_available') return
    setDowngradeConfirmOpen(false)

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
        subtitle="Beebeeb checks for updates at launch and every 4 hours. When one is available, a toast appears in the app."
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
          onClick={() => void desktopUpdateCheck.check()}
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
          {updateCheckBusy ? 'Checking...' : updateCheckError ? 'Try again' : 'Check for updates'}
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
                : `Read the release notes before downgrading to ${updateCheckState.version}.`}
            </div>
            <div style={{ display: 'inline-flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
              <button
                type="button"
                onClick={() => setReleaseNotesOpen(true)}
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
                Release notes
              </button>
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
                View on GitHub
              </button>
              {installSupported && <button
                type="button"
                onClick={openDowngradeConfirmation}
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
              </button>}
            </div>
          </div>
        )}
      </Card>

      {!installSupported && <p>In-app installation is unavailable for this package. {caps?.update_format === 'package_manager' ? 'Update with your package manager.' : 'Download an installer to update.'} <button type="button" onClick={() => void openUrl('https://beebeeb.io/download')}>Downloads</button></p>}

      {installSupported && downgradeConfirmationView && (
        <Modal
          open={downgradeConfirmOpen}
          onClose={() => setDowngradeConfirmOpen(false)}
          title={downgradeConfirmationView.title}
          ariaLabel={downgradeConfirmationView.title}
          maxWidth={520}
          initialFocusRef={downgradeConfirmButtonRef}
          footer={
            <div
              style={{
                display: 'flex',
                justifyContent: 'flex-end',
                gap: 8,
                padding: '12px 20px',
                borderTop: `1px solid ${T.line}`,
                flexShrink: 0,
              }}
            >
              <button
                type="button"
                onClick={() => setDowngradeConfirmOpen(false)}
                style={{
                  height: 32,
                  padding: '0 14px',
                  fontSize: 12,
                  fontFamily: T.fontSans,
                  fontWeight: 600,
                  borderRadius: 6,
                  border: `1px solid ${T.line2}`,
                  background: T.paper,
                  color: T.ink2,
                  cursor: 'pointer',
                }}
              >
                {downgradeConfirmationView.cancelLabel}
              </button>
              <button
                ref={downgradeConfirmButtonRef}
                type="button"
                onClick={() => void handleConfirmDowngrade()}
                disabled={downgradeInstallBusy || updateCheckBusy}
                aria-busy={downgradeInstallBusy}
                style={{
                  height: 32,
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 6,
                  padding: '0 13px',
                  fontSize: 12,
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
                {downgradeConfirmationView.confirmLabel}
              </button>
            </div>
          }
        >
          <div style={{ display: 'grid', gridTemplateColumns: '36px minmax(0, 1fr)', gap: 12, alignItems: 'start' }}>
            <div
              aria-hidden="true"
              style={{
                width: 36,
                height: 36,
                borderRadius: 9,
                background: T.amberBg,
                border: `1px solid ${T.amberDeep}`,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                flexShrink: 0,
              }}
            >
              <NavIcon name="download" size={15} color={T.amberDeep} />
            </div>
            <div style={{ minWidth: 0 }}>
              <p style={{ margin: 0, fontSize: 12.5, color: T.ink2, lineHeight: 1.65 }}>
                {downgradeConfirmationView.message}
              </p>
            </div>
          </div>
        </Modal>
      )}

      {updateCheckState.kind === 'downgrade_available' && (
        <Modal
          open={releaseNotesOpen}
          onClose={() => setReleaseNotesOpen(false)}
          title={`Beebeeb Desktop ${updateCheckState.version}`}
        >
          <div style={{ whiteSpace: 'pre-wrap' as const, fontFamily: T.fontSans, fontSize: 12.5, color: T.ink2, lineHeight: 1.65 }}>
            {updateCheckState.body || 'No release notes were provided for this version.'}
          </div>
        </Modal>
      )}

      <Card style={{ padding: 0, overflow: 'hidden', marginBottom: 16 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '15px 18px', alignItems: 'center', borderBottom: `1px solid ${T.line}` }}>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Release channel</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              Choose which desktop releases {thisDeviceNoun(platform)} receives.
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

      <p style={{ margin: '0 0 16px', fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb checks for updates automatically at startup and every 4 hours.
      </p>
      <p style={{ margin: 0, fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Updates are verified with a minisign signature before installation. No update is applied without your confirmation via the restart toast.
      </p>
    </SettingsSectionShell>
  )
}
