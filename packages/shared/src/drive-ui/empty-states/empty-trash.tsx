// The empty state for the trash view.
import { EmptyState } from './empty-state'

interface EmptyTrashProps {
  onGoToDrive: () => void
}

export function EmptyTrash({ onGoToDrive }: EmptyTrashProps) {
  return (
    <EmptyState
      icon="trash"
      heading="Trash is empty"
      subtitle="Deleted files appear here for 30 days before they are permanently deleted."
      cta={{
        label: 'Back to drive',
        icon: 'folder',
        onClick: onGoToDrive,
        variant: 'default',
      }}
      hint="Tip: right-click any file and choose Trash to move it here."
    />
  )
}
