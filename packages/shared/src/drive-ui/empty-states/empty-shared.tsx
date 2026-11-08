// The empty state for the shared view.
import { EmptyState } from './empty-state'
import type { IconName } from '../../design'

interface EmptySharedProps {
  tab: 'with-me' | 'by-me' | 'pending'
  onGoToDrive: () => void
  /** Called when the user clicks "Share a file" in the with-me empty state. */
  onShareFile?: () => void
}

const config: Record<
  'with-me' | 'by-me' | 'pending',
  { icon: IconName; heading: string; subtitle: string; ctaLabel: string }
> = {
  'with-me': {
    icon: 'users',
    heading: 'No files shared with you yet',
    subtitle:
      'Files shared with you via encrypted links will appear here. Shared files use separate key exchange — the sender must approve your access.',
    ctaLabel: 'Share a file',
  },
  'by-me': {
    icon: 'upload',
    heading: 'No active shares',
    subtitle:
      'Files you share with others will appear here once they accept. Share a file from your vault to get started.',
    ctaLabel: 'Go to drive',
  },
  pending: {
    icon: 'clock',
    heading: 'No pending invites',
    subtitle:
      'Pending share invites and approval requests will appear here. Everything is quiet for now.',
    ctaLabel: 'Go to drive',
  },
}

export function EmptyShared({ tab, onGoToDrive, onShareFile }: EmptySharedProps) {
  const c = config[tab]

  if (tab === 'with-me') {
    return (
      <EmptyState
        icon={c.icon}
        heading={c.heading}
        subtitle={c.subtitle}
        cta={{
          label: c.ctaLabel,
          icon: 'share',
          onClick: onShareFile ?? onGoToDrive,
          variant: 'amber',
        }}
        secondaryCta={{
          label: 'Go to drive',
          icon: 'folder',
          onClick: onGoToDrive,
        }}
      />
    )
  }

  return (
    <EmptyState
      icon={c.icon}
      heading={c.heading}
      subtitle={c.subtitle}
      cta={{
        label: c.ctaLabel,
        icon: 'folder',
        onClick: onGoToDrive,
        variant: 'default',
      }}
    />
  )
}
