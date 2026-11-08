// The empty state for the activity view.
import { EmptyState } from './empty-state'

interface EmptyActivityProps {
  onGoToDrive: () => void
}

export function EmptyActivity({ onGoToDrive }: EmptyActivityProps) {
  return (
    <EmptyState
      icon="shield"
      heading="No security events yet"
      subtitle="Sign-ins, device registrations, password changes, and other security events will appear here."
      cta={{
        label: 'Go to drive',
        icon: 'folder',
        onClick: onGoToDrive,
        variant: 'default',
      }}
      hint="Security events are encrypted per-entry. Only you can read them."
    />
  )
}
