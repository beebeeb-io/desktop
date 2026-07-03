import type { ManualUpdateCheckResult, ReleaseChannel } from '../desktopApi'

export type ManualUpdateCheckState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'up_to_date'; currentVersion: string; channel: ReleaseChannel }
  | { kind: 'update_available'; currentVersion: string; channel: ReleaseChannel; version: string }
  | { kind: 'error'; reason: string }

export type UpdateCheckTone = 'neutral' | 'amber' | 'green'

export interface UpdateCheckViewModel {
  title: string
  detail: string
  chip: string
  tone: UpdateCheckTone
}

const CHANNEL_LABELS: Record<ReleaseChannel, string> = {
  stable: 'Stable',
  beta: 'Beta',
  alpha: 'Alpha',
}

export function releaseChannelLabel(channel: ReleaseChannel): string {
  return CHANNEL_LABELS[channel]
}

export function stateFromManualUpdateResult(result: ManualUpdateCheckResult): ManualUpdateCheckState {
  if (result.status === 'up_to_date') {
    return {
      kind: 'up_to_date',
      currentVersion: result.current_version,
      channel: result.channel,
    }
  }

  return {
    kind: 'update_available',
    currentVersion: result.current_version,
    channel: result.channel,
    version: result.version,
  }
}

export function buildUpdateCheckViewModel(
  state: ManualUpdateCheckState,
  currentVersion: string | null,
  selectedChannel: ReleaseChannel,
): UpdateCheckViewModel {
  if (state.kind === 'checking') {
    const label = releaseChannelLabel(selectedChannel)
    return {
      title: `Checking ${label}...`,
      detail: `Contacting the ${label} channel manifest.`,
      chip: 'Checking',
      tone: 'amber',
    }
  }

  if (state.kind === 'up_to_date') {
    const label = releaseChannelLabel(state.channel)
    return {
      title: `Version ${state.currentVersion} is current`,
      detail: `${label} has no newer desktop update for this install.`,
      chip: 'Up to date',
      tone: 'green',
    }
  }

  if (state.kind === 'update_available') {
    const label = releaseChannelLabel(state.channel)
    return {
      title: `Version ${state.version} is available`,
      detail: `Use the update banner to restart and apply the ${label} update.`,
      chip: 'Update available',
      tone: 'amber',
    }
  }

  if (state.kind === 'error') {
    return {
      title: 'Could not check for updates',
      detail: state.reason,
      chip: 'Check failed',
      tone: 'neutral',
    }
  }

  const versionLabel = currentVersion != null ? `Version ${currentVersion}` : 'Loading version'
  const channelLabel = releaseChannelLabel(selectedChannel)
  return {
    title: versionLabel,
    detail: `Check the ${channelLabel} channel manifest when you want an immediate answer.`,
    chip: 'Not checked',
    tone: 'neutral',
  }
}
