// The Files dialogs an open file uses in place (B07 viewers and the editor, B08 Office): Share (DW-0407) and Version
// history (DW-0413). Files' own components and calls; nothing here keeps a key, a token or a link's secret part.
import { useEffect, useState } from 'react'
import { useDriveSource, type DriveItem, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { navigateTo } from '../../routes'
import { showToast } from '../../chrome/Toast'
import { ShareDialog, VersionsDialog } from '../files/Dialogs'
import { copyShareLink } from '../files/shareLink'

/** Version history over the open file (DW-1524's Versions → DW-0413): Files' dialog, its restore. The restore is queued
 *  for the engine; `onRestored` lets the open file reload when it lands, as a restore (#273 finding 8). */
export function VersionsHost({ item, onClose, onRestored }: { item: DriveItem; onClose: () => void; onRestored?: () => void }) {
  const source = useDriveSource()
  const [versions, setVersions] = useState<VersionEntry[] | null>(null)
  useEffect(() => {
    source.versions(item.id).then(setVersions, () => { showToast({ kind: 'failed', text: 'Couldn’t open Version history. Try again.' }); onClose() })
  }, [source, item.id]) // eslint-disable-line react-hooks/exhaustive-deps -- once per file
  if (!versions) return null
  return (
    <VersionsDialog
      name={item.name} versions={versions} onClose={onClose}
      onRestore={(versionId) => source.restoreVersion(item.id, versionId).then(
        () => {
          onClose()
          // The open file says when it lands; elsewhere Files' words stand.
          if (onRestored) { onRestored(); showToast({ kind: 'done', text: 'Restoring that version. The file reloads when it’s back.' }) } else showToast({ kind: 'done', text: 'Version restored' })
        },
        () => showToast({ kind: 'failed', text: 'Couldn’t restore that version. Nothing was changed.' }),
      )}
    />
  )
}

/** Share from a viewer: Files' Share dialog; Copy link goes to Rust, which makes the link (DW-0407). */
export function ShareHost({ items, onClose }: { items: DriveItem[]; onClose: () => void }) {
  return (
    <ShareDialog
      name={items[0].name} size={items[0].size} count={items.length}
      onCopy={(o) => { onClose(); void copyShareLink(items.map((i) => i.id), o, showToast) }}
      onInvite={() => { onClose(); navigateTo({ section: 'shared' }) }} onClose={onClose}
    />
  )
}
