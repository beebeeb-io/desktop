import type { ManualUpdateCheckResult, ReleaseChannel } from '../desktopApi'

export type ManualUpdateCheckState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'up_to_date'; currentVersion: string; channel: ReleaseChannel }
  | { kind: 'update_available'; currentVersion: string; channel: ReleaseChannel; version: string }
  | {
      kind: 'downgrade_available'
      currentVersion: string
      currentChannel: ReleaseChannel
      channel: ReleaseChannel
      version: string
      body: string
      releaseNotesUrl: string
    }
  | { kind: 'error'; reason: string }

export type UpdateCheckTone = 'neutral' | 'amber' | 'green'

export interface UpdateCheckViewModel {
  title: string
  detail: string
  chip: string
  tone: UpdateCheckTone
}

export interface DowngradeConfirmationViewModel {
  title: string
  message: string
  confirmLabel: string
  cancelLabel: string
}

const CHANNEL_LABELS: Record<ReleaseChannel, string> = {
  stable: 'Stable',
  beta: 'Beta',
  alpha: 'Alpha',
}

export function releaseChannelLabel(channel: ReleaseChannel): string {
  return CHANNEL_LABELS[channel]
}

export function buildDowngradeConfirmationViewModel(
  state: Extract<ManualUpdateCheckState, { kind: 'downgrade_available' }>,
): DowngradeConfirmationViewModel {
  const selectedChannel = releaseChannelLabel(state.channel)
  return {
    title: 'Confirm downgrade',
    message: `Downgrade Beebeeb from ${state.currentVersion} to ${state.version} on the ${selectedChannel} channel? The signed installer will run and Beebeeb will restart if it succeeds.`,
    confirmLabel: `Downgrade to ${state.version}`,
    cancelLabel: 'Cancel',
  }
}

export function stateFromManualUpdateResult(result: ManualUpdateCheckResult): ManualUpdateCheckState {
  if (result.status === 'up_to_date') {
    return {
      kind: 'up_to_date',
      currentVersion: result.current_version,
      channel: result.channel,
    }
  }

  if (result.status === 'downgrade_available') {
    return {
      kind: 'downgrade_available',
      currentVersion: result.current_version,
      currentChannel: result.current_channel,
      channel: result.channel,
      version: result.version,
      body: result.body,
      releaseNotesUrl: result.release_notes_url,
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

  if (state.kind === 'downgrade_available') {
    const selectedLabel = releaseChannelLabel(state.channel)
    const currentLabel = releaseChannelLabel(state.currentChannel)
    return {
      title: `${selectedLabel} is at version ${state.version}`,
      detail: `You are running ${state.currentVersion} from the ${currentLabel} channel. Confirm before downgrading.`,
      chip: 'Downgrade available',
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
