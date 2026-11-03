/**
 * OS integration panels. `ShellOrFinderPanel` picks the macOS Finder reconciler or the
 * Explorer/shell panel; the macOS arm stays behind the platform hook so the Windows build
 * never links it.
 */
import { useRegionLabel } from '../../useRegion'
import { useEffect, useMemo, useState } from 'react'
import { command, commandUnavailableLabel, type FinderInstallState, type SyncStatus } from '../../../desktopApi'
import { shellIntegrationLabel } from '../../settingsNavigation'
import { usePlatform, thisDeviceNoun, type PlatformName } from '../../../platform'
import { T, Card, Chip, PageHeader, PrimaryBtn, useToast } from '../../ui'
import { useFinderSetup } from '../../../finderSetup'
import { FINDER_SETUP_TITLE, finderActionButtonLabel, finderStatusPill } from '../../../finderSetupCopy'
import { SettingsSectionShell } from './primitives'

export type ShellIntegrationState = FinderInstallState

/**
 * The Rust command pair backing the Explorer panel, per platform. Windows registers the sync root
 * as a Cloud Files sync root (`windows_*`) and returns `FinderInstallState` (aliased here as
 * `ShellIntegrationState`). macOS never reaches this panel (`ShellOrFinderPanel` sends it to
 * `MacFinderIntegrationPanel`, which reads the reconciler). Linux has no backing command yet:
 * `null` tells the panel to say so honestly instead of invoking a platform-specific command that
 * can only ever error.
 */
export function shellIntegrationCommandsFor(platform: PlatformName): { state: string; install: string } | null {
  if (platform === 'windows') return { state: 'windows_shell_integration_state', install: 'install_windows_shell_integration' }
  return null
}

export function ExplorerIntegrationPanel({ status }: { status: SyncStatus | null }) {
  const regionLabel = useRegionLabel()
  // Gate everything below on `resolved`: `platform` starts `null` and only
  // becomes a real value once `desktop_platform` answers. Deriving `commands`
  // from `resolved ? platform : null` means `commands` itself is the single
  // gate a shell-integration command can go through — it is `null` (so
  // `enable()`/the state-check effect below both bail) until the real host
  // platform is known. This is what stops the macOS main window (routed with
  // `?platform=windows`) from invoking `windows_shell_integration_state` and
  // surfacing "Windows shell integration is only available on Windows"
  // before flipping to the real Finder panel a moment later (PR #34 review).
  const { name: platformName, resolved } = usePlatform()
  const platform = resolved ? platformName : null
  const label = shellIntegrationLabel(platform)
  // "Explorer integration" / "Finder integration" read fine mid-sentence (proper nouns);
  // the generic Linux fallback reads better lowercase there.
  const midSentenceLabel = platform === 'linux' ? 'file manager integration' : label
  const fileSurfaceName =
    platform === 'macos' ? 'Finder' : platform === 'linux' ? 'your file manager' : platform === 'windows' ? 'File Explorer' : 'your files'
  const commands = useMemo(() => (platform ? shellIntegrationCommandsFor(platform) : null), [platform])
  const actionVerb = 'Enable'
  const actionVerbBusy = 'Enabling...'
  const [state, setState] = useState<ShellIntegrationState | null>(null)
  const [checking, setChecking] = useState(false)
  const [busy, setBusy] = useState(false)
  const { showToast } = useToast()

  const refresh = async () => {
    if (!commands) return
    const r = await command<ShellIntegrationState>(commands.state)
    if (r.ok) {
      setState(r.value)
    } else {
      showToast({
        variant: 'error',
        title: `Couldn’t check ${midSentenceLabel}`,
        message: r.unsupported ? commandUnavailableLabel(commands.state) : r.reason,
      })
    }
  }

  // Does not run until `resolved === true` (via `commands`, which is `null`
  // for an unresolved platform) — no command invocation, no toast, while the
  // real host platform is still unknown.
  useEffect(() => {
    if (!commands) return
    let cancelled = false
    setChecking(true)
    void (async () => {
      const r = await command<ShellIntegrationState>(commands.state)
      if (cancelled) return
      if (r.ok) setState(r.value)
      else showToast({
        variant: 'error',
        title: `Couldn’t check ${midSentenceLabel}`,
        message: r.unsupported ? commandUnavailableLabel(commands.state) : r.reason,
      })
      setChecking(false)
    })()
    return () => { cancelled = true }
  }, [showToast, midSentenceLabel, commands])

  // Windows/Linux: a failed enable is a toast and nothing is painted from the saved
  // `finder_install_last_error` (the Windows first-run flow saves one that Settings has never shown).
  // macOS has no install action at all since spec 2026-10-06: it renders MacFinderIntegrationPanel.
  const enable = async () => {
    if (!commands) return
    setBusy(true)
    const r = await command<ShellIntegrationState>(commands.install)
    setBusy(false)
    if (r.ok) {
      setState(r.value)
      void refresh()
      return
    }
    showToast({
      variant: 'error',
      title: `Couldn’t ${actionVerb.toLowerCase()} ${midSentenceLabel}`,
      message: r.unsupported ? commandUnavailableLabel(commands.install) : r.reason,
    })
  }

  const active = state?.installed === true
  const deviceNoun = platform ? thisDeviceNoun(platform) : 'this device'
  const statusText = !resolved
    ? 'Checking...'
    : !commands
      ? `${label} isn’t available on ${deviceNoun} yet.`
      : checking
        ? 'Checking...'
        : active
          ? `Active. Beebeeb is registered as a sync folder on ${deviceNoun}.`
          : `Not set up yet on ${deviceNoun}.`

  return (
    <SettingsSectionShell>
      <PageHeader
        title={label}
        subtitle={`Beebeeb appears in ${fileSurfaceName} as a sync folder. Files are encrypted on ${deviceNoun} before they leave. ${regionLabel}.`}
      />

      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>{fileSurfaceName} location</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>{statusText}</div>
          {platform === 'windows' && (
            <div title={status?.sync_root ?? undefined} style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, marginTop: 3, overflowWrap: 'anywhere' }}>
              {status == null ? 'Sync folder unavailable' : status.sync_root ?? 'Not configured on this PC yet'}
            </div>
          )}
        </div>
        {commands && !checking && !active && (
          <PrimaryBtn onClick={() => void enable()} disabled={busy}>{busy ? actionVerbBusy : actionVerb}</PrimaryBtn>
        )}
        {commands && !checking && active && <Chip tone="green">Active</Chip>}
      </Card>
    </SettingsSectionShell>
  )
}

/** Dispatches the main window's integration panel: macOS reads the reconciler (spec 2026-10-06). */
export function ShellOrFinderPanel({ status }: { status: SyncStatus | null }) {
  const { name, resolved } = usePlatform()
  return resolved && name === 'macos' ? <MacFinderIntegrationPanel /> : <ExplorerIntegrationPanel status={status} />
}

/**
 * macOS: the reconciler's state, read-only. The load, the `finder-setup-changed` subscription and
 * the failed-action toast come from the one shared hook (lead ruling 7b). No install action exists
 * here (R5): Beebeeb adds itself to Finder, and this panel says where that stands. A failed SETUP
 * gates Finder, so it is the one inline notice below the card (never also a toast); a failed
 * ACTION gates nothing, so the hook raises it as a toast.
 */
export function MacFinderIntegrationPanel() {
  const regionLabel = useRegionLabel()
  const finder = useFinderSetup()
  const presentation = finder.presentation
  const alert = presentation.kind === 'notice' && presentation.tone === 'alert'
  // A notice's card line is the notice's own pill label (keyed on the presentation, FT-I2); every
  // other state says its line. A loaded Missing says its one resting sentence (FA-I2), and before the
  // first answer the line is empty: no ad-hoc "Checking" string (M6).
  const detail = presentation.kind === 'notice' ? (finderStatusPill(finder.load)?.label ?? '') : presentation.line
  return (
    <SettingsSectionShell>
      <PageHeader title="Finder integration" subtitle={`Beebeeb appears in Finder as a sync folder. Files are encrypted on this Mac before they leave. ${regionLabel}.`} />
      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>{FINDER_SETUP_TITLE}</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>{detail}</div>
        </div>
        {presentation.kind === 'ready' && <Chip tone="green">Active</Chip>}
        {/* The state could not be read: this reads it again (`retry`). It is not the reconciler's
            `try_again`, which only exists inside a failure notice. */}
        {presentation.kind === 'unavailable' && <PrimaryBtn onClick={() => void finder.retry()}>{presentation.actionLabel}</PrimaryBtn>}
      </Card>
      {presentation.kind === 'notice' && (
        <div
          role={alert ? 'alert' : 'status'}
          data-error-surface={alert ? 'finder-setup' : undefined}
          style={{
            marginTop: 10,
            padding: '10px 12px',
            background: alert ? 'var(--err-bg)' : T.paper2,
            border: alert ? '1px solid var(--err-line)' : `1px solid ${T.line2}`,
            borderRadius: 8,
            color: T.ink,
            fontSize: 12,
            lineHeight: 1.5,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
          }}
        >
          <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>{presentation.sentence}</span>
          {/* No action for an unconfirmed engine stop: only a relaunch helps (row 9). */}
          {presentation.action && (
            <PrimaryBtn onClick={() => void finder.run(presentation.action!)}>
              {finderActionButtonLabel(presentation.action, presentation.actionLabel, finder.copied)}
            </PrimaryBtn>
          )}
        </div>
      )}
      {/* Row 15: what a failed Try again said (Rust's fixed sentence, with its remedy), neutrally. */}
      {finder.actionNote && (
        <div role="status" style={{ marginTop: 10, fontSize: 12, lineHeight: 1.5, color: T.ink2 }}>
          {finder.actionNote}
        </div>
      )}
    </SettingsSectionShell>
  )
}
