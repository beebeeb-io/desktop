/**
 * The two Settings pieces that read the account on their own (task 1683 slice 5; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8): General › "From your account" and Account › "Storage details…".
 * Each is a component with its own state, so each has its own tests (`tests/macSettingsExtras.test.tsx`) and the tabs that
 * host them are tested with a marker in their place.
 *
 * One failure, one surface: a switch that did not save is a toast (a one-off action) and goes back to what is saved; a
 * read that failed is one inline alert with Try again. Nothing here shows a raw error.
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported above, so the
 * tests can execute them with the component harness.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  accountNotificationPreferences,
  accountStorageBreakdown,
  accountUpdateNotificationPreferences,
  commandUnavailableLabel,
  type NotificationPreferenceValues,
} from './desktopApi'
import { STORAGE_DETAILS_LARGEST, STORAGE_DETAILS_SUBTITLE, storageDetails, type StorageDetails } from './macSettingsModel'
import { Btn, Note, SettingRow, SettingsGroup, ToggleRow } from './macSettingsParts'
import { ACCOUNT_NOTIFICATION_PREF_META, type AccountNotificationPrefKey } from './windows/notificationPreferenceMeta'
import { Modal, useToast } from './windows/ui'

// ── General › From your account ─────────────────────────────────────────────

type PrefsState = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; values: NotificationPreferenceValues }

/**
 * The account's own notification preferences (New device sign-in, Share received, File updated, Storage warning, Backup
 * complete): stored with the account, so the same five follow the person to every device. A press shows at once and is
 * sent as exactly that one preference; the server's answer is what the switches then show. While a write runs the switches
 * wait, so two presses cannot cross.
 */
export function AccountPreferencesGroup() {
  const { showToast } = useToast()
  const [state, setState] = useState<PrefsState>({ status: 'loading' })
  const [busy, setBusy] = useState(false)

  const load = useCallback(async () => {
    setState({ status: 'loading' })
    const result = await accountNotificationPreferences()
    setState(result.ok ? { status: 'ready', values: result.value.preferences } : { status: 'failed' })
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const toggle = async (key: AccountNotificationPrefKey, next: boolean) => {
    if (state.status !== 'ready' || busy) return
    const before = state.values
    setState({ status: 'ready', values: { ...before, [key]: next } })
    setBusy(true)
    const result = await accountUpdateNotificationPreferences({ [key]: next })
    setBusy(false)
    if (result.ok) {
      setState({ status: 'ready', values: result.value.preferences })
      return
    }
    setState({ status: 'ready', values: before })
    showToast({
      variant: 'error',
      title: 'Couldn’t save that setting',
      message: result.unsupported ? commandUnavailableLabel('account_update_notification_preferences') : result.reason,
    })
  }

  const values = state.status === 'ready' ? state.values : null

  return (
    <SettingsGroup title="Account notifications">
      {state.status === 'failed' ? (
        <Note
          kind="alert"
          surface="account-prefs-load"
          title="Couldn’t load these settings"
          actions={<Btn onClick={() => void load()}>Try again</Btn>}
        />
      ) : (
        ACCOUNT_NOTIFICATION_PREF_META.map((meta) => (
          <ToggleRow
            key={meta.key}
            name={meta.key}
            label={meta.label}
            hint={meta.hint}
            on={values?.[meta.key] === true}
            disabled={values === null || busy}
            onChange={(next) => void toggle(meta.key, next)}
          />
        ))
      )}
    </SettingsGroup>
  )
}

// ── Account › Storage details ───────────────────────────────────────────────

type DetailsState = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; details: StorageDetails }

/**
 * Where the storage goes, by type and by the largest files: the Windows shell's Insights, in a sheet. Nothing is read until
 * the sheet is opened, and it is read again each time it opens, so the figures are the ones of now. Each read has a number,
 * and only the latest one may answer: a slow read from before the sheet was closed, or from before Try again, does not land
 * on the figures of a later one. Closing resets the sheet, so the next opening starts from "Loading…", not from the last figures.
 */
export function StorageDetailsSheet({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [state, setState] = useState<DetailsState>({ status: 'loading' })
  const latest = useRef(0)

  const load = useCallback(async () => {
    const mine = ++latest.current
    setState({ status: 'loading' })
    const result = await accountStorageBreakdown(STORAGE_DETAILS_LARGEST)
    if (mine !== latest.current) return
    setState(result.ok ? { status: 'ready', details: storageDetails(result.value) } : { status: 'failed' })
  }, [])

  useEffect(() => {
    if (open) {
      void load()
      return
    }
    latest.current += 1
    setState({ status: 'loading' })
  }, [open, load])

  const details = state.status === 'ready' ? state.details : null

  return (
    <Modal open={open} onClose={onClose} title="Storage details" maxWidth={520}>
      <p className="ms-sheet-copy">{STORAGE_DETAILS_SUBTITLE}</p>
      {state.status === 'loading' ? <div className="ms-loading" role="status">Loading…</div> : null}
      {state.status === 'failed' ? (
        <Note
          kind="alert"
          surface="storage-details"
          title="Couldn’t load storage details"
          actions={<Btn onClick={() => void load()}>Try again</Btn>}
        />
      ) : null}
      {details && details.empty ? <p className="ms-sheet-copy">Nothing is stored yet.</p> : null}
      {details && !details.empty ? (
        <div className="ms-details">
          <p className="ms-details-total">{details.total}</p>
          <SettingsGroup title="By type">
            {details.types.map((type) => (
              <SettingRow key={type.key} name={`type-${type.key}`} label={type.label} hint={`${type.size} · ${type.files}`} />
            ))}
          </SettingsGroup>
          {details.largest.length > 0 ? (
            <SettingsGroup title="Largest files">
              {details.largest.map((file) => (
                <SettingRow
                  key={file.id}
                  name={`file-${file.id}`}
                  label={<span className="ms-mono ms-mono--wrap">{file.path}</span>}
                  hint={file.hint}
                />
              ))}
            </SettingsGroup>
          ) : null}
        </div>
      ) : null}
    </Modal>
  )
}
