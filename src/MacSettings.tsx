/**
 * The macOS Settings sections (task 1683 slice 4; spec `docs/specs/2026-09-30-macos-menubar-popover.md` section 5),
 * hosted since task 1683 slice 1 by the Settings pane of the one Beebeeb window (`MacApp.tsx`; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.8, System Settings style). The bodies are the shipped ones,
 * unchanged: only the toolbar tab strip went, replaced by the window's sidebar list, and 1887's flow object and its
 * listeners moved up to the window's shell.
 *
 * Layout contract (spec section 8; `macSettings.css`, pinned by tests/macSettingsLayout.test.ts and the Chromium run
 * in tests/render-mac-settings.mjs): only the content pane scrolls; grid tracks are `minmax(0, 1fr)`; every flex text
 * block is `min-width: 0`; text that can hold user data wraps anywhere.
 *
 * One failure, one surface (spec section 7): a Finder failure is the inline block under its row, never also a toast;
 * the manual update check answers inline in its own row because this window does not mount the update toast. A
 * one-off action that gates nothing (a switch that did not save, a failed lock) is a toast, as the house rule says.
 *
 * Slice 5 adds Appearance and "From your account" to General, "Storage details…" and the web row to Account, and the release
 * channel, the downgrade row and the speed test to About (Security is its own file, `MacSecurity.tsx`). About offers a
 * download, never an install: the App Sandbox cannot install an update in place (task 2176).
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported above,
 * so `tests/macSettingsTabs.test.tsx` can execute them with the component harness and read what a user would read.
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react'
import {
  accountSubscription,
  BILLING_URL,
  command,
  commandUnavailableLabel,
  lockVault,
  openUrl,
  popoverSnapshot,
  type DesktopConfig,
  type DesktopTheme,
  type CommandResult,
  type MacosIntegrationResetResult,
  type SessionActionOutcome,
  type Subscription,
  type VaultItem,
} from './desktopApi'
import type { PopoverSnapshot } from './popoverContract'
import { clearSignOutWarning, heldSignOutWarning, holdSignOutWarning, subscribeSignOutWarning } from './accountSession'
import { useFinderSetup } from './finderSetup'
import { finderActionButtonLabel } from './finderSetupCopy'
import { SUPPORT_BUNDLE_DETAIL, SUPPORT_BUNDLE_SAVED_TITLE, supportBundleSavedMessage, type ProblemReportResult } from './diagnosticsCopy'
import {
  ACCOUNT_SETTINGS_URL,
  accountInitial,
  APPEARANCE_OPTIONS,
  appearanceHint,
  appearanceValue,
  downgradeRow,
  finderHint,
  finderRow,
  HELP_URL,
  keepCountLabel,
  keepOnMac,
  monoReason,
  NOTIFICATION_ROWS,
  planLine,
  preservedFilesNote,
  RELEASE_CHANNELS,
  releaseChannelOf,
  REPAIR_BODY,
  REPAIR_TITLE,
  repairNote,
  SPEED_TEST_HINT,
  speedOptions,
  speedTestSummary,
  storageLine,
  updateRow,
} from './macSettingsModel'
import { Btn, ChoiceSelect, Note, Select, SettingRow, SettingsGroup, SettingsIcon, Switch, ToggleRow } from './macSettingsParts'
import { AccountPreferencesGroup, StorageDetailsSheet } from './MacSettingsExtras'
import { desktopSpeedTest } from './macSpeedTest'
import { updateOffer } from './macUpdateOffer'
import {
  A0_TITLE,
  ASK_CANCEL,
  ASK_SIGN_OUT,
  askLines,
  P0_TITLE,
  P2_HINT,
  P3_STOP,
  progressLine,
  stripCode,
  type AskCounts,
  type EndReason,
  type NotSending,
} from './signOutCopy'
import { SignOutFlow, type FlowOutcome, type FlowView } from './signOutFlow'
import { residencySummary } from './windows/dataResidencySettingsModel'
import { useDataResidency } from './windows/useDataResidency'
import { KeptFolderNotes, useKeptFolders } from './keptFolderRows'
import { Modal, useToast } from './windows/ui'
import { desktopUpdateCheck } from './windows/manualUpdateCheck'
import { setDesktopThemePreference } from './windows/theme'

// ── Shared state: the desktop config ────────────────────────────────────────

type ConfigState = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; config: DesktopConfig }

export interface SettingsConfig {
  state: ConfigState
  /** Apply a change now and write it; on a failed write say so once and show what is on disk. */
  save: (patch: Partial<DesktopConfig>) => Promise<void>
  reload: () => Promise<void>
}

/**
 * Notifications and the speed limits are fields of one config object that other writers change too (the tray
 * pauses sync while this window is open). `set_desktop_config` merges only the fields it is sent, so each write
 * carries ONLY the field its own change touched, never the object this window read when it opened: changing a
 * speed after a pause from the tray cannot write the old `pause_sync` back (task 1889, fix round 1 M5). The window
 * owns one copy for what it shows, writes are queued in order, and a later change cannot rewrite what an earlier
 * write sends. A failed write is a toast, and the config is read back so the switch shows what is really saved,
 * not what was clicked; the changes still queued behind the failure are applied on top of what that read found, so
 * a later change survives an earlier one failing.
 */
export function useSettingsConfig(): SettingsConfig {
  const { showToast } = useToast()
  const [state, setState] = useState<ConfigState>({ status: 'loading' })
  const latest = useRef<DesktopConfig | null>(null)
  const queue = useRef<Promise<void>>(Promise.resolve())
  /** Changes queued behind the write that is running. Each is written as exactly this patch. */
  const pending = useRef<Array<{ patch: Partial<DesktopConfig> }>>([])

  const read = useCallback(async (): Promise<boolean> => {
    const result = await command<DesktopConfig>('get_desktop_config')
    if (!result.ok) return false
    latest.current = result.value
    setState({ status: 'ready', config: result.value })
    return true
  }, [])

  const reload = useCallback(async () => {
    setState({ status: 'loading' })
    if (!(await read())) setState({ status: 'failed' })
  }, [read])

  useEffect(() => {
    void reload()
  }, [reload])

  // The window wears the SAVED theme (Settings › General › Appearance). The config is the one source: a choice shows at once
  // because `save` changes the config first, and a write that failed, read back as the old theme, puts the old look back.
  // It lives here, with the config, so it holds on every section and not only while General is open. The first reading
  // applies too: `main.tsx` applied the theme from its own read of the config when the window opened, and that read can
  // differ from this one; applying the same theme twice changes nothing.
  const theme = state.status === 'ready' ? appearanceValue(state.config.theme) : null
  useEffect(() => {
    if (theme !== null) setDesktopThemePreference(theme)
  }, [theme])

  const save = useCallback(
    (patch: Partial<DesktopConfig>): Promise<void> => {
      const base = latest.current
      if (base === null) return Promise.resolve()
      const next = { ...base, ...patch }
      latest.current = next
      setState({ status: 'ready', config: next })
      const entry = { patch }
      pending.current.push(entry)
      const write = async () => {
        const result = await command<void>('set_desktop_config', { config: entry.patch })
        pending.current = pending.current.filter((queued) => queued !== entry)
        if (result.ok) return
        showToast({
          variant: 'error',
          title: 'Couldn’t save that setting',
          message: result.unsupported ? commandUnavailableLabel('set_desktop_config') : result.reason,
        })
        if (!(await read())) {
          setState({ status: 'failed' })
          return
        }
        // What is on disk lacks the failed patch and every queued one. The queued patches still go out
        // as they are; the window shows them on top of what the read found, and the failed change as
        // unsaved instead of claiming it.
        const onDisk = latest.current
        if (onDisk === null) return
        let current = onDisk
        for (const queued of pending.current) current = { ...current, ...queued.patch }
        latest.current = current
        setState({ status: 'ready', config: current })
      }
      queue.current = queue.current.then(write, write)
      return queue.current
    },
    [read, showToast],
  )

  return { state, save, reload }
}

function ConfigLoadFailed({ onRetry }: { onRetry: () => void }) {
  return (
    <SettingsGroup>
      <Note
        kind="alert"
        surface="settings-config"
        title="Couldn’t load these settings"
        actions={<Btn onClick={onRetry}>Try again</Btn>}
      />
    </SettingsGroup>
  )
}

// ── General ─────────────────────────────────────────────────────────────────

export function GeneralTab({ settings, signedIn }: { settings: SettingsConfig; signedIn: boolean }) {
  const { showToast } = useToast()
  const [login, setLogin] = useState<boolean | null>(null)
  const [loginUnreadable, setLoginUnreadable] = useState(false)

  useEffect(() => {
    let cancelled = false
    void command<boolean>('autostart_enabled').then((result) => {
      if (cancelled) return
      if (result.ok) setLogin(result.value)
      else setLoginUnreadable(true)
    })
    return () => {
      cancelled = true
    }
  }, [])

  // A one-off action that gates nothing: a failure is a toast (house rule).
  const toggleLogin = async () => {
    const result = await command<boolean>('toggle_autostart')
    if (result.ok) {
      setLogin(result.value)
      return
    }
    showToast({
      variant: 'error',
      title: 'Couldn’t change the login setting',
      message: result.unsupported ? commandUnavailableLabel('toggle_autostart') : result.reason,
    })
  }

  const config = settings.state.status === 'ready' ? settings.state.config : null
  const theme = appearanceValue(config?.theme)
  const loaded = config !== null

  return (
    <>
      <SettingsGroup>
        <ToggleRow
          tall
          name="login"
          label="Open Beebeeb at login"
          hint={loginUnreadable ? 'Couldn’t read this setting.' : 'Starts in the menu bar. No window opens.'}
          on={login === true}
          disabled={login === null}
          onChange={() => void toggleLogin()}
        />
        <SettingRow
          name="appearance"
          tall
          label="Theme"
          hint={appearanceHint(theme)}
          control={
            <ChoiceSelect
              name="appearance"
              value={theme}
              options={APPEARANCE_OPTIONS}
              disabled={!loaded}
              onChange={(next) => void settings.save({ theme: next as DesktopTheme })}
            />
          }
        />
      </SettingsGroup>
      {settings.state.status === 'failed' ? (
        <ConfigLoadFailed onRetry={() => void settings.reload()} />
      ) : (
        <SettingsGroup title="On this device">
          {NOTIFICATION_ROWS.map((row) => (
            <ToggleRow
              key={row.key}
              name={row.key}
              label={row.label}
              hint={row.hint}
              on={config?.[row.key] === true}
              disabled={config === null}
              onChange={(next) => void settings.save({ [row.key]: next })}
            />
          ))}
        </SettingsGroup>
      )}
      {/* Stored with the account, so there is nothing to show before sign-in (spec §7.8, "Signed out"). */}
      {signedIn ? <AccountPreferencesGroup /> : null}
    </>
  )
}

// ── Account ─────────────────────────────────────────────────────────────────

// ── Account > Where your files are stored (task 1919) ──────────────────────

/**
 * The storage-location choice. The rows are the API's list (`GET /me/region` -> `choices`) through the
 * one hook the Windows panel uses: "Europe" is the default (any of our European data centers, the
 * server places the data), a live country is a row to pick, a coming-soon country is listed and cannot
 * be picked (task 2050: by country, never a city). This window keeps no list, city or country of its
 * own, so a country the server adds or retires changes these rows with no release. While a country is
 * chosen, "Only store in <country>" is a switch under the rows.
 *
 * One failure, one surface: a list that did not load is the inline block with Try again; a choice the
 * server refused is a toast (a one-off action that gates nothing) and the row goes back to what is saved.
 */
function DataResidencyGroup() {
  const { showToast } = useToast()
  const { loading, loadError, view, retry, select, setStrict } = useDataResidency()

  const chooseStrict = async (next: boolean) => {
    const result = await setStrict(next)
    if (result.status !== 'error') return
    showToast({
      variant: 'error',
      title: 'Couldn’t change where files are stored',
      message: result.message || 'Please try again.',
    })
  }

  const choose = async (id: string) => {
    const result = await select(id)
    if (result?.status !== 'error') return
    showToast({
      variant: 'error',
      title: 'Couldn’t change where files are stored',
      message: result.message || 'Please try again.',
    })
  }

  if (loading) {
    return (
      <SettingsGroup title="Storage location">
        <SettingRow name="region-loading" label="Loading…" />
      </SettingsGroup>
    )
  }
  const summary = residencySummary(view)
  if (loadError !== null || (summary === null && !view.preferenceUnavailable)) {
    return (
      <SettingsGroup title="Storage location">
        <Note
          kind="alert"
          surface="data-residency-load"
          title="Couldn’t load storage locations"
          actions={<Btn onClick={() => void retry()}>Try again</Btn>}
        />
      </SettingsGroup>
    )
  }

  return (
    <SettingsGroup title="Storage location">
      <SettingRow
        name="region-summary"
        label={view.preferenceUnavailable ? 'Your chosen location isn’t available right now.' : summary}
        hint={view.preferenceUnavailable ? 'New files go wherever Beebeeb places them by default.' : undefined}
      />
      <div role="radiogroup" aria-label="Where your files are stored" className="ms-choices">
        {view.items.map((item) => (
          <button
            key={item.id}
            type="button"
            role="radio"
            className="ms-choice"
            data-row={`region-${item.id}`}
            aria-checked={item.selected}
            aria-labelledby={`ms-region-${item.id}-label`}
            aria-describedby={item.detail ? `ms-region-${item.id}-hint` : undefined}
            disabled={item.disabled}
            onClick={() => void choose(item.id)}
          >
            <span className="ms-radio" aria-hidden="true" />
            <span className="ms-row-text">
              <span className="ms-row-label" id={`ms-region-${item.id}-label`}>
                {item.displayName}
                {item.isDefault ? <span className="ms-tag">Default</span> : null}
                {item.comingSoon ? <span className="ms-tag">Coming soon</span> : null}
              </span>
              {item.detail ? (
                <span className="ms-row-hint" id={`ms-region-${item.id}-hint`}>
                  {item.detail}
                </span>
              ) : null}
            </span>
          </button>
        ))}
      </div>
      {view.pinned ? (
        <ToggleRow
          name="region-strict"
          label={view.pinned.strictLabel}
          hint={view.pinned.strictHint}
          on={view.strict}
          disabled={view.saving}
          onChange={(next) => void chooseStrict(next)}
        />
      ) : null}
      {view.onlyOneRegion ? null : (
        <Note kind="status">Existing files stay where they are. Only new files use the location you choose.</Note>
      )}
    </SettingsGroup>
  )
}

type AccountLoad = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; snapshot: PopoverSnapshot }
type AccountBusy = 'lock_vault' | 'unlock_vault' | null

/**
 * The sign-out runs through the window's one flow (spec 2026-10-10 §5.1), which the root owns and hands in with its
 * view and a count of its outcomes: this tab mounts on every visit, so it builds no flow and consumes no request.
 */
export function AccountTab({ flow, flowView, flowOutcomes }: { flow: SignOutFlow; flowView: FlowView; flowOutcomes: number }) {
  const { showToast } = useToast()
  const [load, setLoad] = useState<AccountLoad>({ status: 'loading' })
  const [plan, setPlan] = useState<Subscription | null | undefined>(undefined)
  const [busy, setBusy] = useState<AccountBusy>(null)
  const [detailsOpen, setDetailsOpen] = useState(false)
  // FB-24: a Lock or a sign-out that happened but could not confirm one step. Rust's sentence, shown
  // as a neutral line (never under "Couldn’t …"), until the next action. A sign-out's sentence is held
  // outside the session boundary, because the sign-out remounts this tab (accountSession.ts).
  const [actionNote, setActionNote] = useState<string | null>(null)
  const signOutNote = useSyncExternalStore(subscribeSignOutWarning, heldSignOutWarning, heldSignOutWarning)

  const refresh = useCallback(async () => {
    const result = await popoverSnapshot(1)
    setLoad(result.ok ? { status: 'ready', snapshot: result.value } : { status: 'failed' })
  }, [])

  // Read again after each sign-out outcome as well: a read, so running it again on a mount is harmless.
  useEffect(() => {
    void refresh()
  }, [refresh, flowOutcomes])

  const account = load.status === 'ready' ? load.snapshot.account : null
  const signedIn = account?.logged_in === true
  const unlocked = account?.vault_unlocked === true

  // The plan needs the session; it is read again after an unlock.
  useEffect(() => {
    if (!signedIn || !unlocked) {
      setPlan(undefined)
      return
    }
    let cancelled = false
    void accountSubscription().then((result) => {
      if (!cancelled) setPlan(result.ok ? result.value : null)
    })
    return () => {
      cancelled = true
    }
  }, [signedIn, unlocked])

  // One action at a time. An Err means it did not happen: one error toast. An Ok of a Lock may carry a
  // warning (FB-24): its sentence becomes the neutral note. The account is read again after ANY result
  // (M4), so the tab never keeps showing a state the action changed. The sign-out is the flow's.
  const run = async (name: NonNullable<AccountBusy>, send: () => Promise<CommandResult<unknown>>, failTitle: string) => {
    setBusy(name)
    setActionNote(null)
    clearSignOutWarning()
    const result = await send()
    setBusy(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: failTitle,
        message: result.unsupported ? commandUnavailableLabel(name) : result.reason,
      })
    }
    await refresh()
    return result
  }

  const lock = async () => {
    const result = await run('lock_vault', lockVault, 'Couldn’t lock the vault')
    if (result.ok) setActionNote((result.value as SessionActionOutcome).warning?.sentence ?? null)
  }

  const shownNote = actionNote ?? signOutNote
  const note = shownNote ? (
    <SettingsGroup>
      <Note kind="status">{shownNote}</Note>
    </SettingsGroup>
  ) : null

  // The note stays up while the tab loads: a sign-out's remount loads this tab again from scratch.
  if (load.status === 'loading') {
    return (
      <>
        {note}
        <div className="ms-loading" role="status">Loading…</div>
      </>
    )
  }
  if (load.status === 'failed' || account === null) {
    return (
      <>
        {note}
        <SettingsGroup>
          <Note
            kind="alert"
            surface="account-load"
            title="Couldn’t load your account"
            actions={<Btn onClick={() => void refresh()}>Try again</Btn>}
          />
        </SettingsGroup>
      </>
    )
  }

  if (!account.logged_in) {
    return (
      <>
        {note}
        <SettingsGroup>
          <SettingRow name="signed-out" tall label="You’re signed out" hint="Sign in from the Beebeeb menu to start syncing." />
        </SettingsGroup>
      </>
    )
  }

  const storage = storageLine(load.snapshot.storage)
  const sessionEnded = load.snapshot.phase === 'session_ended' || account.auth_expired
  const planText = !unlocked ? null : plan === undefined ? null : plan === null ? 'Plan details aren’t available right now.' : planLine(plan)

  return (
    <>
      {note}
      <SettingsGroup>
        <div className="ms-account">
          <div className="ms-avatar" aria-hidden="true">
            {accountInitial(account.email)}
          </div>
          <div className="ms-account-text">
            <div className="ms-account-email">{account.email ?? 'Signed in'}</div>
            {planText ? <div className="ms-account-plan">{planText}</div> : null}
            {storage ? (
              <div className="ms-storage">
                <div
                  className="ms-bar"
                  role="progressbar"
                  aria-label="Storage used"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(storage.percent)}
                  data-red={storage.red ? 'true' : undefined}
                >
                  <div className="ms-bar-fill" style={{ width: `${storage.percent}%` }} />
                </div>
                <span className="ms-storage-label">{storage.label}</span>
                {/* The Insights content, in a sheet: reading it needs the vault open (as the plan does). */}
                {unlocked ? (
                  <button type="button" className="ms-link" onClick={() => setDetailsOpen(true)}>
                    Storage details…
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
          {unlocked ? (
            <Btn onClick={() => void openUrl(BILLING_URL)}>{plan?.plan.toLowerCase() === 'free' ? 'Upgrade' : 'Manage plan'}</Btn>
          ) : null}
        </div>
      </SettingsGroup>

      <SettingsGroup title="Vault">
        {sessionEnded ? (
          <SettingRow name="vault" tall label="Your session ended" hint="Sign in again to keep syncing." />
        ) : unlocked ? (
          <SettingRow
            name="vault"
            tall
            label={
              <span className="ms-with-glyph">
                <span className="ms-lock-glyph">
                  <SettingsIcon name="unlock" size={15} />
                </span>
                Vault is unlocked
              </span>
            }
            hint="Locking stops sync until you enter your password again."
            control={
              <Btn disabled={busy === 'lock_vault'} onClick={() => void lock()}>
                {busy === 'lock_vault' ? 'Locking…' : 'Lock now'}
              </Btn>
            }
          />
        ) : (
          <SettingRow
            name="vault"
            tall
            label={
              <span className="ms-with-glyph">
                <span className="ms-lock-glyph">
                  <SettingsIcon name="lock" size={15} />
                </span>
                Vault is locked
              </span>
            }
            hint="Sync is paused until you unlock it."
            control={
              <Btn primary disabled={busy === 'unlock_vault'} onClick={() => void run('unlock_vault', () => command<void>('unlock_vault'), 'Couldn’t unlock the vault')}>
                {busy === 'unlock_vault' ? 'Unlocking…' : 'Unlock'}
              </Btn>
            }
          />
        )}
      </SettingsGroup>

      <DataResidencyGroup />

      <SettingsGroup>
        <SettingRow
          name="web"
          tall
          label="Password, email and billing"
          hint="Changed on beebeeb.io, in your browser."
          control={
            <Btn onClick={() => void openUrl(ACCOUNT_SETTINGS_URL)}>
              <SettingsIcon name="external" size={13} />
              Open beebeeb.io
            </Btn>
          }
        />
        <SettingRow
          name="signout"
          tall
          label="Sign out of this Mac"
          hint="You’ll need your password to sign back in."
          control={
            <Btn disabled={flowView.kind !== 'idle'} onClick={() => flow.open()}>
              Sign out…
            </Btn>
          }
        />
      </SettingsGroup>

      <StorageDetailsSheet open={detailsOpen} onClose={() => setDetailsOpen(false)} />

      <ConfirmSheet
        open={flowView.kind === 'confirm'}
        title="Sign out of this Mac?"
        confirmLabel="Sign out"
        danger
        onCancel={() => flow.cancel()}
        onConfirm={() => void flow.confirm()}
      >
        Sync stops until you sign in again.
      </ConfirmSheet>
      {flowView.kind === 'sending' ? <SendingSheet changesLeft={flowView.changesLeft} onStop={() => void flow.stopWaiting()} /> : null}
      {flowView.kind === 'ask' ? (
        <UnsentAskSheet ask={flowView.ask} reason={flowView.reason} onCancel={() => flow.cancel()} onSignOut={() => void flow.signOutAtAsk()} />
      ) : null}
    </>
  )
}

/**
 * A small in-window question with Cancel and one confirming action. Esc and the close button cancel.
 *
 * `danger` renders the confirm as the destructive red (`ms-btn--danger`): the ruling
 * (2026-10-02) gives it to Sign out, which stops sync and is not offered as reversible, and
 * keeps Repair amber because it is reversible. Confirm order stays Cancel → confirm.
 *
 * Enter confirms only when the confirm button itself is focused: the handler lives on that
 * button, so on open — where focus is the close button — a stray Return natively activates
 * Cancel and never reaches the confirm. `preventDefault` keeps the browser's native
 * Enter-activates-a-focused-button click from firing the confirm a second time.
 */
export function ConfirmSheet({
  open,
  title,
  confirmLabel,
  onCancel,
  onConfirm,
  danger = false,
  children,
}: {
  open: boolean
  title: string
  confirmLabel: string
  onCancel: () => void
  onConfirm: () => void
  danger?: boolean
  children: string
}) {
  const onConfirmKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    onConfirm()
  }
  return (
    <Modal
      open={open}
      onClose={onCancel}
      title={title}
      maxWidth={400}
      footer={
        <div className="ms-sheet-footer">
          <Btn onClick={onCancel}>Cancel</Btn>
          <Btn primary={!danger} danger={danger} onKeyDown={onConfirmKeyDown} onClick={onConfirm}>
            {confirmLabel}
          </Btn>
        </div>
      }
    >
      <p className="ms-sheet-copy">{children}</p>
    </Modal>
  )
}

/** Spec 2026-10-10 §11, P0-P3: the send phase. Esc is "Stop waiting". The count never reads 0. */
function SendingSheet({ changesLeft, onStop }: { changesLeft: number; onStop: () => void }) {
  return (
    <Modal open onClose={onStop} title={P0_TITLE} maxWidth={400} footer={<div className="ms-sheet-footer"><Btn onClick={onStop}>{P3_STOP}</Btn></div>}>
      <p className="ms-sheet-copy" aria-live="polite">{progressLine(changesLeft)}</p>
      <p className="ms-sheet-copy">{P2_HINT}</p>
    </Modal>
  )
}

/** Spec 2026-10-10 §11, A0-A5: the ask. Dialog 3's keyboard rule: Enter on open is Cancel, Esc cancels. */
function UnsentAskSheet({ ask, reason, onCancel, onSignOut }: { ask: AskCounts; reason: EndReason | NotSending | null; onCancel: () => void; onSignOut: () => void }) {
  const lines = askLines(ask, reason, false)
  const onConfirmKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    onSignOut()
  }
  return (
    <Modal
      open
      onClose={onCancel}
      title={A0_TITLE}
      maxWidth={440}
      footer={
        <div className="ms-sheet-footer">
          <Btn onClick={onCancel}>{ASK_CANCEL}</Btn>
          <Btn danger onKeyDown={onConfirmKeyDown} onClick={onSignOut}>{ASK_SIGN_OUT}</Btn>
        </div>
      }
    >
      {lines.paragraphs.slice(0, lines.foldersAfter).map((line) => <p key={line} className="ms-sheet-copy">{line}</p>)}
      {lines.foldersLine ? <p className="ms-sheet-copy">{lines.foldersLine}</p> : null}
      {/* Each path a sheet paragraph in mono, as Dialog 3's ask is drawn (design/hifi/macos-settings-dialogs.html). */}
      {lines.folders.map((folder) => <p key={folder} className="ms-sheet-copy" style={{ fontFamily: 'var(--font-mono)' }}>{folder}</p>)}
      {lines.paragraphs.slice(lines.foldersAfter).map((line) => <p key={line} className="ms-sheet-copy">{line}</p>)}
    </Modal>
  )
}

// ── Sync ────────────────────────────────────────────────────────────────────

type RepairPhase = 'idle' | 'confirming' | 'busy'

export function SyncTab({ settings }: { settings: SettingsConfig }) {
  const { showToast } = useToast()
  // The Finder row is the reconciler's state, read and followed by the one shared hook (lead
  // ruling 7b): it loads, follows `finder-setup-changed` (no polling), and toasts a failed ACTION.
  // A failed SETUP is part of the state: it gates Finder, so it is the inline notice under the
  // row and never also a toast. Nothing here adds Beebeeb to Finder by hand (R5): the reconciler does.
  const finder = useFinderSetup()
  const [repair, setRepair] = useState<RepairPhase>('idle')
  const [repairFailed, setRepairFailed] = useState(false)
  const [repairResult, setRepairResult] = useState<string | null>(null)
  const [tree, setTree] = useState<VaultItem[] | null>(null)
  const [treeLoadFailed, setTreeLoadFailed] = useState(false)
  const [chooser, setChooser] = useState(false)
  const [savingFolder, setSavingFolder] = useState<string | null>(null)

  const loadTree = useCallback(async () => {
    setTreeLoadFailed(false)
    const result = await command<VaultItem[]>('list_remote_tree')
    if (result.ok) setTree(result.value)
    else setTreeLoadFailed(true)
  }, [])

  useEffect(() => {
    void loadTree()
  }, [loadTree])

  // Task 1882 and task 1887: the two "files were kept" rows. One hook and one component, shared with Home's "Needs your
  // attention" (two-surface spec §8.3), so both read the same state and a dismiss in either place clears both.
  const keptFolders = useKeptFolders()
  const { loadKept, setKept } = keptFolders

  const runRepair = async () => {
    setRepair('busy')
    setRepairFailed(false)
    setRepairResult(null)
    const result = await command<MacosIntegrationResetResult>('reset_macos_integration')
    setRepair('idle')
    if (!result.ok) {
      setRepairFailed(true)
      await finder.retry()
      return
    }
    setRepairResult(repairNote(result.value))
    // On a Mac, Repair saves the kept folder (if any) for the row on its own (`remember_kept_folder`, under the
    // config-write lock), and only best-effort: a failed save is logged, Repair still succeeds, and it raises no
    // alert. The row reads the record back. So when no other folder is saved, this fallback covers both a save that
    // failed and a record that cannot be read back just now: the folder this repair reported is still shown. (Rebase
    // re-review Minor 5: this said a failed save fails Repair and raises the alert, which was main's Repair, not a
    // Mac's.)
    await loadKept()
    const reported = preservedFilesNote(result.value)
    if (reported) setKept((current) => current ?? reported.path)
    // The reconciler adds Beebeeb back by itself and says so through finder-setup-changed; this
    // read is the safety net that keeps the row from claiming the pre-repair "Added".
    await finder.retry()
  }

  const toggleFolder = async (id: string, pinned: boolean) => {
    setSavingFolder(id)
    const result = await command<unknown>('set_recursive_pin', { itemId: id, pinned })
    setSavingFolder(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: pinned ? 'Couldn’t keep that folder on this Mac' : 'Couldn’t stop keeping that folder',
        message: result.unsupported ? commandUnavailableLabel('set_recursive_pin') : result.reason,
      })
      return
    }
    setTree((current) => (current === null ? current : withPinned(current, id, pinned)))
  }

  const row = finderRow(finder.load.status === 'loaded' ? finder.load.view : null, finder.load.status === 'unavailable', finder.refusal)
  const keep = keepOnMac(tree, treeLoadFailed)
  const config = settings.state.status === 'ready' ? settings.state.config : null

  let finderControl = null
  if (row.kind === 'added') {
    finderControl = (
      <>
        <span className="ms-added">
          <span className="ms-ok-disc">
            <SettingsIcon name="check" size={11} strokeWidth={2.2} />
          </span>
          Added
        </span>
        <Btn disabled={repair === 'busy'} onClick={() => setRepair('confirming')}>
          {repair === 'busy' ? 'Repairing…' : 'Repair…'}
        </Btn>
      </>
    )
  } else if (row.kind === 'adding') {
    finderControl = <Btn disabled>Adding…</Btn>
  } else if (row.kind === 'unavailable') {
    // The state could not be read: this Try again reads it again. It is NOT the reconciler's
    // retry, which only exists inside a failure notice below (two different "Try again"s).
    finderControl = <Btn onClick={() => void finder.retry()}>Try again</Btn>
  }

  let keepControl = null
  if (keep.kind === 'ready') {
    keepControl = (
      <>
        <span className="ms-count">{keepCountLabel(keep.pinned)}</span>
        <Btn disabled={keep.folders.length === 0} onClick={() => setChooser(true)}>
          Choose folders…
        </Btn>
      </>
    )
  } else if (keep.kind === 'failed') {
    keepControl = <Btn onClick={() => void loadTree()}>Try again</Btn>
  } else if (keep.kind === 'unreported') {
    keepControl = <Btn disabled>Choose folders…</Btn>
  }

  const keepHint =
    keep.kind === 'failed'
      ? 'Couldn’t load your folders.'
      : keep.kind === 'unreported'
        ? 'Not available in this version yet.'
        : 'Everything else downloads when you open it.'

  return (
    <>
      <SettingsGroup>
        <SettingRow
          name="finder"
          tall
          label="Beebeeb in Finder"
          hint={finderHint(row)}
          control={finderControl}
        />
        {row.kind === 'notice' ? (
          <Note
            kind={row.tone}
            surface="finder-setup"
            reason={row.tone === 'alert' ? monoReason(row.reason) : null}
            actions={row.action ? <Btn onClick={() => void finder.run(row.action!)}>{finderActionButtonLabel(row.action, row.actionLabel, finder.copied)}</Btn> : null}
          >
            {row.sentence}
          </Note>
        ) : null}
        {finder.actionNote ? <Note kind="status">{finder.actionNote}</Note> : null}
        {repairFailed ? (
          <Note kind="alert" surface="finder-repair" title="Couldn’t repair Beebeeb in Finder">
            Nothing was changed that you need to undo. Try again.
          </Note>
        ) : null}
        {repairResult ? <Note kind="status">{repairResult}</Note> : null}
        <KeptFolderNotes folders={keptFolders} />
        <SettingRow
          name="keep"
          tall
          label="Keep on this Mac"
          hint={keepHint}
          control={keepControl}
        />
      </SettingsGroup>

      {settings.state.status === 'failed' ? (
        <ConfigLoadFailed onRetry={() => void settings.reload()} />
      ) : (
        <SettingsGroup title="Speed">
          <SettingRow
            name="upload"
            label="Upload"
            control={
              <Select
                name="upload"
                value={config?.upload_kbps_limit ?? 0}
                options={speedOptions(config?.upload_kbps_limit ?? 0)}
                disabled={config === null}
                onChange={(value) => void settings.save({ upload_kbps_limit: value })}
              />
            }
          />
          <SettingRow
            name="download"
            label="Download"
            control={
              <Select
                name="download"
                value={config?.download_kbps_limit ?? 0}
                options={speedOptions(config?.download_kbps_limit ?? 0)}
                disabled={config === null}
                onChange={(value) => void settings.save({ download_kbps_limit: value })}
              />
            }
          />
        </SettingsGroup>
      )}

      <ConfirmSheet
        open={repair === 'confirming'}
        title={REPAIR_TITLE}
        confirmLabel="Repair"
        onCancel={() => setRepair('idle')}
        onConfirm={() => void runRepair()}
      >
        {REPAIR_BODY}
      </ConfirmSheet>

      <Modal open={chooser} onClose={() => setChooser(false)} title="Keep on this Mac" maxWidth={440}>
        <p className="ms-sheet-copy">Folders you choose stay on this Mac. Everything else downloads when you open it.</p>
        <div className="ms-card ms-folder-list">
          {keep.kind === 'ready'
            ? keep.folders.map((folder, index) => (
                <div className="ms-row" key={folder.id} data-row={`folder-${index}`}>
                  <div className="ms-row-text">
                    <div className="ms-row-label" id={`ms-folder-${index}-label`}>
                      {folder.name}
                    </div>
                    {folder.where ? <div className="ms-row-hint">{folder.where}</div> : null}
                  </div>
                  <div className="ms-row-control">
                    <Switch
                      name={`folder-${index}`}
                      on={folder.pinned}
                      disabled={savingFolder === folder.id}
                      onChange={(next) => void toggleFolder(folder.id, next)}
                    />
                  </div>
                </div>
              ))
            : null}
        </div>
      </Modal>
    </>
  )
}

function withPinned(items: VaultItem[], id: string, pinned: boolean): VaultItem[] {
  return items.map((item) => ({
    ...item,
    pinned: item.id === id ? pinned : item.pinned,
    children: item.children ? withPinned(item.children, id, pinned) : item.children,
  }))
}

// ── About ───────────────────────────────────────────────────────────────────

export function AboutTab({ settings, signedIn }: { settings: SettingsConfig; signedIn: boolean }) {
  const { showToast } = useToast()
  const update = useSyncExternalStore(desktopUpdateCheck.subscribe, desktopUpdateCheck.getSnapshot, desktopUpdateCheck.getSnapshot)
  const [version, setVersion] = useState<string | null>(null)
  const [exporting, setExporting] = useState(false)
  const [notesOpen, setNotesOpen] = useState(false)
  const speed = useSyncExternalStore(desktopSpeedTest.subscribe, desktopSpeedTest.getSnapshot, desktopSpeedTest.getSnapshot)
  const downgrade = update.kind === 'downgrade_available' ? update : null
  const hasDowngrade = downgrade !== null
  // What this Mac is offered for the update it found: the newest stable image for a stable one, that release's page for
  // another channel (`macUpdateOffer.ts`).
  const offer = update.kind === 'update_available' ? updateOffer(update.channel, update.version) : null

  useEffect(() => {
    let cancelled = false
    void command<string>('app_version').then((result) => {
      if (!cancelled && result.ok) setVersion(result.value)
    })
    return () => {
      cancelled = true
    }
  }, [])

  // The notes belong to the offer on screen: when the offer goes (a new check, another channel) they close with it.
  useEffect(() => {
    if (!hasDowngrade) setNotesOpen(false)
  }, [hasDowngrade])

  // Task 2176: the App Sandbox cannot install an update in place, so a Mac is sent to the download. There is no install
  // here, and no command that would try one.
  const downloadUpdate = async () => {
    if (offer === null) return
    const result = await openUrl(offer.url)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: offer.stableBuild ? 'Couldn’t open the download' : 'Couldn’t open the release page',
        message: result.unsupported ? commandUnavailableLabel('open_url') : result.reason,
      })
    }
  }

  // `report_problem` writes the bundle and reveals it; the copy of what is in it is the
  // exact copy task 1685 pinned to the Rust export (src/diagnosticsCopy.ts).
  const exportBundle = async () => {
    setExporting(true)
    const result = await command<ProblemReportResult>('report_problem')
    setExporting(false)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t save the support bundle',
        message: result.unsupported ? commandUnavailableLabel('report_problem') : result.reason,
      })
      return
    }
    showToast({
      variant: result.value.email_opened ? 'success' : 'warning',
      title: SUPPORT_BUNDLE_SAVED_TITLE,
      message: supportBundleSavedMessage(result.value.path, result.value.email_opened),
    })
  }

  // A one-off action that gates nothing: a failed test is a toast and the row goes back to its hint. The test lives in the
  // window's own store (`macSpeedTest.ts`), so leaving this page does not lose it.
  const runTest = async () => {
    const failure = await desktopSpeedTest.start()
    if (failure !== null) showToast({ variant: 'error', title: 'Couldn’t run the speed test', message: failure })
  }

  const config = settings.state.status === 'ready' ? settings.state.config : null
  const channel = releaseChannelOf(config?.release_channel)
  const channelHint = RELEASE_CHANNELS.find((entry) => entry.value === channel)?.hint

  // Another channel is another answer: what the last check said was about the old one.
  const chooseChannel = (next: string) => {
    const id = releaseChannelOf(next)
    if (id === channel) return
    desktopUpdateCheck.reset()
    void settings.save({ release_channel: id })
  }

  const row = updateRow(update, version)
  const downgradeOffer = downgradeRow(update)

  return (
    <>
      <SettingsGroup footnote="Beebeeb checks for updates at launch and every 4 hours.">
        <SettingRow
          name="version"
          tall
          label="Beebeeb for Mac"
          hint={row.hint}
          control={
            row.action === 'download' && offer ? (
              <Btn primary onClick={() => void downloadUpdate()}>
                {offer.label}
              </Btn>
            ) : (
              <Btn disabled={row.action === 'checking'} onClick={() => void desktopUpdateCheck.check()}>
                Check for updates
              </Btn>
            )
          }
        />
        {settings.state.status === 'failed' ? null : (
          <SettingRow
            name="channel"
            tall
            label="Release channel"
            hint={channelHint}
            control={<ChoiceSelect name="channel" value={channel} options={RELEASE_CHANNELS} disabled={config === null} onChange={chooseChannel} />}
          />
        )}
        {downgradeOffer && downgrade ? (
          <SettingRow
            name="downgrade"
            tall
            label={downgradeOffer.label}
            hint={downgradeOffer.hint}
            control={
              <>
                <Btn onClick={() => setNotesOpen(true)}>Release notes</Btn>
                <Btn onClick={() => void openUrl(downgrade.releaseNotesUrl)}>
                  <SettingsIcon name="external" size={13} />
                  View release
                </Btn>
              </>
            }
          />
        ) : null}
      </SettingsGroup>
      {settings.state.status === 'failed' ? <ConfigLoadFailed onRetry={() => void settings.reload()} /> : null}
      <SettingsGroup title="Help">
        <SettingRow
          name="help"
          label="Get help"
          hint="Opens our support page in your browser."
          control={
            <button type="button" className="ms-iconbtn" aria-label="Get help, opens in your browser" onClick={() => void openUrl(HELP_URL)}>
              <SettingsIcon name="external" size={14} />
            </button>
          }
        />
        {signedIn ? (
          <>
            <SettingRow
              name="speed"
              tall
              label="Test connection speed"
              hint={speed.phase === 'running' ? 'Testing… this takes 30 to 90 seconds.' : SPEED_TEST_HINT}
              control={
                <Btn disabled={speed.phase === 'running'} onClick={() => void runTest()}>
                  {speed.phase === 'running' ? 'Running…' : 'Run test'}
                </Btn>
              }
            />
            {speed.phase === 'done' ? <Note kind="status" mono>{speedTestSummary(speed.result)}</Note> : null}
          </>
        ) : null}
        <SettingRow
          name="bundle"
          tall
          label="Export a support bundle"
          hint={SUPPORT_BUNDLE_DETAIL}
          control={
            <Btn disabled={exporting} onClick={() => void exportBundle()}>
              {exporting ? 'Saving…' : 'Export…'}
            </Btn>
          }
        />
      </SettingsGroup>
      {downgrade ? (
        <Modal open={notesOpen} onClose={() => setNotesOpen(false)} title={`Beebeeb ${downgrade.version}`} maxWidth={440}>
          <p className="ms-sheet-copy ms-notes">{downgrade.body.trim() || 'No release notes were provided for this version.'}</p>
        </Modal>
      ) : null}
    </>
  )
}

// ── The window ──────────────────────────────────────────────────────────────

/** How the window shows a sign-out's outcome (§5.1): a failure as one toast, its code stripped (§11); a completed
 *  sign-out's warning held for the Account tab's note, outside the session boundary (accountSession.ts). */
export function showSignOutOutcome(outcome: FlowOutcome, showToast: (toast: { variant: 'error'; title: string; message: string }) => void) {
  if (outcome.kind === 'failed') {
    showToast({ variant: 'error', title: 'Couldn’t sign out', message: stripCode(outcome.message) })
  } else {
    holdSignOutWarning(outcome.warning)
  }
}
