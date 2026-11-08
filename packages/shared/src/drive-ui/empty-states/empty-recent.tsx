// The empty state for the recent-files view.
import { EmptyState } from './empty-state'

interface EmptyRecentProps {
  onUpload: () => void
}

export function EmptyRecent({ onUpload }: EmptyRecentProps) {
  return (
    <EmptyState
      icon="clock"
      heading="No recent activity"
      subtitle="Files you upload or modify will show up here, sorted by date. Start by uploading your first file."
      cta={{
        label: 'Upload a file',
        icon: 'upload',
        onClick: onUpload,
      }}
    />
  )
}
