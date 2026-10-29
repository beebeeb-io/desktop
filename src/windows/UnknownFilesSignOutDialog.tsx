import { Modal, PrimaryBtn, T } from './ui'
import { unknownFilesSignOutCopy } from '../signOutUnknownFilesCopy'

/**
 * Task 2191 (spec D11, Decision (lead, 2026-10-26)): before a Windows sign-out removes files
 * from before the last update that hold bytes on this PC, the person chooses: keep copies (the
 * default) or delete them.
 */
export function UnknownFilesSignOutDialog({
  count,
  folder,
  onCancel,
  onChoose,
}: {
  count: number
  folder: string
  onCancel: () => void
  onChoose: (choice: 'keep' | 'delete') => void
}) {
  const copy = unknownFilesSignOutCopy(count, folder)
  const plain = {
    padding: '7px 14px',
    fontSize: 12,
    fontFamily: T.fontSans,
    fontWeight: 500,
    borderRadius: 6,
    border: `1px solid ${T.line2}`,
    background: T.paper2,
    color: T.ink,
    cursor: 'pointer',
  } as const
  return (
    <Modal
      open
      onClose={onCancel}
      title={copy.title}
      maxWidth={520}
      footer={
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', flexWrap: 'wrap' }}>
          <button style={plain} onClick={onCancel}>
            {copy.cancelLabel}
          </button>
          <button style={plain} onClick={() => onChoose('delete')}>
            {copy.deleteLabel}
          </button>
          <PrimaryBtn onClick={() => onChoose('keep')}>{copy.keepLabel}</PrimaryBtn>
        </div>
      }
    >
      <p style={{ fontSize: 12.5, color: T.ink, lineHeight: 1.6, margin: '0 0 10px' }}>{copy.body}</p>
      <p style={{ fontSize: 12, color: T.ink2, lineHeight: 1.6, margin: '0 0 8px' }}>{copy.keepNote}</p>
      <p style={{ fontSize: 12, color: T.ink2, lineHeight: 1.6, margin: 0 }}>{copy.deleteNote}</p>
    </Modal>
  )
}
