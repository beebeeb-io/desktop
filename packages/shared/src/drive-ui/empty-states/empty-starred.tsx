// The empty state for the starred view.
import { EmptyState } from './empty-state'

interface EmptyStarredProps {
  onGoToDrive: () => void
}

export function EmptyStarred({ onGoToDrive }: EmptyStarredProps) {
  return (
    <EmptyState
      icon="star"
      heading="No starred files"
      subtitle="Star important files to find them quickly. Right-click any file and choose Star, or use the details panel."
      cta={{
        label: 'Go to drive',
        icon: 'folder',
        onClick: onGoToDrive,
        variant: 'default',
      }}
    />
  )
}
