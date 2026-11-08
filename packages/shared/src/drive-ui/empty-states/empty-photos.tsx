// The empty state for the photos view.
import { EmptyState } from './empty-state'

interface EmptyPhotosProps {
  onUpload: () => void
  onGoToDrive: () => void
}

export function EmptyPhotos({ onUpload, onGoToDrive }: EmptyPhotosProps) {
  return (
    <EmptyState
      icon="image"
      heading="No photos yet"
      subtitle="Enable camera backup in the Beebeeb iOS app to automatically back up your photos here — or upload photos directly."
      cta={{
        label: 'Upload photos',
        icon: 'upload',
        onClick: onUpload,
      }}
      secondaryCta={{
        label: 'Go to drive',
        icon: 'folder',
        onClick: onGoToDrive,
      }}
      hint="Photos are end-to-end encrypted. Only you can decrypt them."
    >
      <a
        href="https://apps.apple.com/app/id6766666400"
        target="_blank"
        rel="noopener noreferrer"
        className="mt-3 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-paper border border-line text-[11.5px] text-ink-2 hover:bg-paper-2 transition-colors"
      >
        Get Beebeeb for iOS
      </a>
    </EmptyState>
  )
}
