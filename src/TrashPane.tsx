/**
 * The Trash section of the Beebeeb window (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.7: "From the Windows shell, copy unchanged"; hi-fi section 3). Ported from `WindowsApp.tsx` `TrashView`, which
 * Windows keeps: the same three commands (`desktop_trash_list`, `desktop_trash_restore`, and `desktop_confirm_action`
 * then `desktop_trash_delete_permanently` behind a password), drawn as the window's rows and sheet. Until task 1717
 * wires Finder's Put Back, this is the Mac's restore path.
 *
 * "Restore" puts the file back through sync and the row leaves; a failure is one toast, and the list stays. "Delete…"
 * opens the "Delete permanently" sheet: a permanent delete needs the account password, which the server turns into a
 * short-lived confirmation token (`desktop_confirm_action`); no token, no delete. A wrong password keeps the sheet,
 * says so once inside it, and sends nothing further. The sheet opens on its close button (§11), so Enter on open is
 * Cancel; once a password is typed, Enter in that field confirms, which is a deliberate act. While the delete runs
 * nothing closes the sheet.
 *
 * "Refresh" is the window's toolbar button; it reaches this pane as `refreshSignal`, a number that grows with each
 * press. An answer to an older load never lands over a newer one.
 *
 * The component is a plain function declaration that reads only its props, hooks and the bindings imported here, so
 * `tests/trashPane.test.tsx` can execute it with the component harness.
 */
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import {
  commandUnavailableLabel,
  desktopConfirmAction,
  desktopTrashDeletePermanently,
  desktopTrashList,
  desktopTrashRestore,
  type DesktopTrashItem,
} from './desktopApi'
import { Btn, Note, SettingsIcon } from './macSettingsParts'
import { TRASH_EMPTY_HINT, TRASH_EMPTY_TITLE, TRASH_INTRO, trashDeleteSheetText, trashRowLine } from './trashModel'
import { Modal, useToast } from './windows/ui'

type TrashLoad =
  | { status: 'loading'; items: DesktopTrashItem[]; reason: null }
  | { status: 'ready'; items: DesktopTrashItem[]; reason: null }
  | { status: 'failed'; items: DesktopTrashItem[]; reason: string }

export function TrashPane({ refreshSignal }: { refreshSignal: number }) {
  const { showToast } = useToast()
  const [load, setLoad] = useState<TrashLoad>({ status: 'loading', items: [], reason: null })
  const [busyId, setBusyId] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<DesktopTrashItem | null>(null)
  const [password, setPassword] = useState('')
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)
  const loadRequest = useRef(0)

  const reload = async () => {
    const current = ++loadRequest.current
    // A refresh keeps the list it has until the answer comes; only a first load or a retry shows "Loading…".
    setLoad((previous) => (previous.status === 'ready' ? previous : { status: 'loading', items: [], reason: null }))
    const result = await desktopTrashList()
    if (loadRequest.current !== current) return
    if (result.ok) setLoad({ status: 'ready', items: result.value, reason: null })
    else setLoad({ status: 'failed', items: [], reason: result.unsupported ? commandUnavailableLabel('desktop_trash_list') : result.reason })
  }

  useEffect(() => {
    void reload()
  }, [refreshSignal])

  // A row leaves after a Restore or a delete. Any list that is still out was asked for before that and would put the row
  // back, so it is dropped: the list on screen is the last answer, minus what was just done to it.
  const without = (id: string) => {
    loadRequest.current += 1
    setLoad((previous) => ({ ...previous, items: previous.items.filter((row) => row.id !== id) }) as TrashLoad)
  }

  const restore = async (item: DesktopTrashItem) => {
    setBusyId(item.id)
    const result = await desktopTrashRestore(item.id, item.name)
    setBusyId(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Restore failed',
        message: result.unsupported ? commandUnavailableLabel('desktop_trash_restore') : result.reason,
      })
      return
    }
    without(item.id)
  }

  const askToDelete = (item: DesktopTrashItem) => {
    setPendingDelete(item)
  }

  // Closing the sheet, by Cancel, Esc, the close button or a finished delete, forgets the password and the error: the
  // sheet that opens next starts clean, and a typed password is not kept in memory behind a closed sheet.
  const cancelDelete = () => {
    setPendingDelete(null)
    setDeleteError(null)
    setPassword('')
  }

  const confirmDelete = async () => {
    if (pendingDelete === null || deleting || password.length === 0) return
    setDeleting(true)
    setDeleteError(null)
    const confirmed = await desktopConfirmAction(password)
    if (!confirmed.ok) {
      setDeleting(false)
      setDeleteError(confirmed.unsupported ? commandUnavailableLabel('desktop_confirm_action') : confirmed.reason)
      return
    }
    const deleted = await desktopTrashDeletePermanently(pendingDelete.id, confirmed.value.confirmation_token)
    setDeleting(false)
    if (!deleted.ok) {
      setDeleteError(deleted.unsupported ? commandUnavailableLabel('desktop_trash_delete_permanently') : deleted.reason)
      return
    }
    without(pendingDelete.id)
    cancelDelete()
  }

  const onPasswordKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter' || password.length === 0) return
    event.preventDefault()
    void confirmDelete()
  }

  const now = new Date()
  let body
  if (load.status === 'loading') {
    body = (
      <div className="ms-loading" role="status">
        Loading…
      </div>
    )
  } else if (load.status === 'failed') {
    body = (
      <Note kind="alert" surface="trash-load" title="Couldn’t load Trash" actions={<Btn onClick={() => void reload()}>Try again</Btn>}>
        {load.reason}
      </Note>
    )
  } else {
    body = (
      <>
        <p className="mw-intro">{TRASH_INTRO}</p>
        <div className="mw-card">
          {load.items.length === 0 ? (
            <div className="mw-empty">
              <div className="mw-row-title">{TRASH_EMPTY_TITLE}</div>
              <div className="mw-row-hint">{TRASH_EMPTY_HINT}</div>
            </div>
          ) : (
            load.items.map((item) => {
              const busy = busyId === item.id
              return (
                <div className="mw-row" data-row="trash" key={item.id}>
                  <span className="mw-tile" aria-hidden="true">
                    <SettingsIcon name={item.is_folder ? 'folder' : 'file'} size={16} />
                  </span>
                  <div className="mw-row-text">
                    <div className="mw-row-title">{item.name}</div>
                    <div className="mw-row-hint">{trashRowLine(item, now)}</div>
                  </div>
                  <Btn disabled={busy} onClick={() => void restore(item)}>
                    Restore
                  </Btn>
                  <Btn disabled={busy} onClick={() => askToDelete(item)}>
                    Delete…
                  </Btn>
                </div>
              )
            })
          )}
        </div>
      </>
    )
  }

  return (
    <div className="mw-content">
      {body}
      <Modal
        open={pendingDelete !== null}
        onClose={deleting ? () => {} : cancelDelete}
        title="Delete permanently"
        ariaLabel="Delete permanently"
        maxWidth={420}
        footer={
          <div className="ms-sheet-footer">
            <Btn disabled={deleting} onClick={cancelDelete}>
              Cancel
            </Btn>
            <Btn danger disabled={deleting || password.length === 0} onClick={() => void confirmDelete()}>
              {deleting ? 'Deleting…' : 'Delete permanently'}
            </Btn>
          </div>
        }
      >
        <div className="mw-sheet-stack">
          <p className="ms-sheet-copy">{pendingDelete === null ? '' : trashDeleteSheetText(pendingDelete.name)}</p>
          <input
            type="password"
            className="mw-search"
            value={password}
            onChange={(event) => setPassword(event.currentTarget.value)}
            onKeyDown={onPasswordKey}
            placeholder="Password"
            aria-label="Password"
            autoComplete="current-password"
          />
          {deleteError ? (
            <Note kind="alert" surface="trash-delete">
              {deleteError}
            </Note>
          ) : null}
        </div>
      </Modal>
    </div>
  )
}
