/**
 * The macOS Settings window (task 1683 slice 4; spec
 * `docs/specs/2026-09-30-macos-menubar-popover.md` section 5, artboards j, k, l, m).
 *
 * One window, four tabs (General, Account, Sync, About), sized to its content. It replaces the
 * 680 x 540 sidebar window whose layout broke in screenshot 3. It is mounted ONLY at
 * `?window=settings-v2&platform=macos` (main.tsx) until slice 6 flips macOS to it; no config
 * creates a window for it yet, so nothing a user sees today changes.
 *
 * Layout contract (spec section 8; `macSettings.css`, pinned by tests/macSettingsLayout.test.ts
 * and the Chromium run in tests/render-mac-settings.mjs): the shell is `height: 100vh;
 * overflow: hidden` and only the content pane scrolls; grid tracks are `minmax(0, 1fr)`; every
 * flex text block is `min-width: 0`; text that can hold user data wraps anywhere.
 *
 * One failure, one surface (spec section 7): a Finder failure is the inline block under its
 * row, never also a toast; the manual update check answers inline in its own row because this
 * window does not mount the update toast. A one-off action that gates nothing (a switch that
 * did not save, a failed lock) is a toast, as the house rule says.
 *
 * The components are plain function declarations that read only their props, hooks and the
 * bindings imported above, so `tests/macSettingsTabs.test.tsx` can execute them with the
 * component harness and read what a user would read.
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react'
import {
  accountSubscription,
  BILLING_URL,
  clearSession,
  command,
  commandUnavailableLabel,
  lockVault,
  openUrl,
  popoverSnapshot,
  type DesktopConfig,
  type CommandResult,
  type MacosIntegrationResetResult,
  type SessionActionOutcome,
  type Subscription,
  type VaultItem,
} from './desktopApi'
import type { PopoverSnapshot } from './popoverContract'
import { clearSignOutWarning, heldSignOutWarning, holdSignOutWarning, subscribeSignOutWarning } from './accountSession'
import { subscribeKeptFolderChanged, useFinderSetup } from './finderSetup'
import { finderActionButtonLabel } from './finderSetupCopy'
import { SUPPORT_BUNDLE_DETAIL, SUPPORT_BUNDLE_SAVED_TITLE, supportBundleSavedMessage, type ProblemReportResult } from './diagnosticsCopy'
import {
  accountInitial,
  finderHint,
  finderRow,
  HELP_URL,
  keepCountLabel,
  keepOnMac,
  KEPT_FOLDER_ROW_SENTENCE,
  keptFolderAfterDismiss,
  monoReason,
  NOTIFICATION_ROWS,
  planLine,
  preservedFilesNote,
  REPAIR_BODY,
  REPAIR_TITLE,
  repairNote,
  SETTINGS_TABS,
  settingsTabFromLocation,
  speedOptions,
  storageLine,
  tabAfterKey,
  updateRow,
  type DismissKeptFolderResult,
  type SettingsTab,
} from './macSettingsModel'
import { Btn, Note, Select, SettingRow, SettingsGroup, SettingsIcon, Switch, ToggleRow } from './macSettingsParts'
import { Modal, useToast } from './windows/ui'
import { listen } from '@tauri-apps/api/event'
import { connectNativeUpdateMenu, desktopUpdateCheck } from './windows/manualUpdateCheck'

// ── Shared state: the desktop config ────────────────────────────────────────

type ConfigState = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; config: DesktopConfig }

export interface SettingsConfig {
  state: ConfigState
  /** Apply a change now and write it; on a failed write say so once and show what is on disk. */
  save: (patch: Partial<DesktopConfig>) => Promise<void>
  reload: () => Promise<void>
}

/**
 * Notifications and the speed limits are fields of one config object that `set_desktop_config`
 * replaces as a whole, so the window owns one copy: two quick changes cannot overwrite each
 * other with a stale object. Writes are queued in order, and each write carries the snapshot
 * its own change produced, so a later change cannot rewrite what an earlier write sends. A
 * failed write is a toast, and the config is read back so the switch shows what is really
 * saved, not what was clicked; the writes still queued behind the failure are rebased onto
 * what that read found, so a later change survives an earlier one failing.
 */
function useSettingsConfig(): SettingsConfig {
  const { showToast } = useToast()
  const [state, setState] = useState<ConfigState>({ status: 'loading' })
  const latest = useRef<DesktopConfig | null>(null)
  const queue = useRef<Promise<void>>(Promise.resolve())
  /** Changes queued behind the write that is running, with the snapshot each will send. */
  const pending = useRef<Array<{ patch: Partial<DesktopConfig>; next: DesktopConfig }>>([])

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

  const save = useCallback(
    (patch: Partial<DesktopConfig>): Promise<void> => {
      const base = latest.current
      if (base === null) return Promise.resolve()
      const next = { ...base, ...patch }
      latest.current = next
      setState({ status: 'ready', config: next })
      const entry = { patch, next }
      pending.current.push(entry)
      const write = async () => {
        const result = await command<void>('set_desktop_config', { config: entry.next })
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
        // What is on disk lacks the failed patch and every queued one: rebase the still-queued
        // snapshots onto the config the read brought back, so the later changes still go out
        // and the window shows the failed change as unsaved instead of claiming it.
        const onDisk = latest.current
        if (onDisk === null) return
        let current = onDisk
        for (const queued of pending.current) {
          current = { ...current, ...queued.patch }
          queued.next = current
        }
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

function GeneralTab({ settings }: { settings: SettingsConfig }) {
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
      </SettingsGroup>
      {settings.state.status === 'failed' ? (
        <ConfigLoadFailed onRetry={() => void settings.reload()} />
      ) : (
        <SettingsGroup title="Tell me about">
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
    </>
  )
}

// ── Account ─────────────────────────────────────────────────────────────────

type AccountLoad = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; snapshot: PopoverSnapshot }
type AccountBusy = 'lock_vault' | 'unlock_vault' | 'clear_session' | null

function AccountTab() {
  const { showToast } = useToast()
  const [load, setLoad] = useState<AccountLoad>({ status: 'loading' })
  const [plan, setPlan] = useState<Subscription | null | undefined>(undefined)
  const [busy, setBusy] = useState<AccountBusy>(null)
  const [confirmSignOut, setConfirmSignOut] = useState(false)
  // FB-24: a Lock or a sign-out that happened but could not confirm one step. Rust's sentence, shown
  // as a neutral line (never under "Couldn’t …"), until the next action. A sign-out's sentence is held
  // outside the session boundary, because the sign-out remounts this tab (accountSession.ts).
  const [actionNote, setActionNote] = useState<string | null>(null)
  const signOutNote = useSyncExternalStore(subscribeSignOutWarning, heldSignOutWarning, heldSignOutWarning)

  const refresh = useCallback(async () => {
    const result = await popoverSnapshot(1)
    setLoad(result.ok ? { status: 'ready', snapshot: result.value } : { status: 'failed' })
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

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

  // One action at a time. An Err means it did not happen: one error toast. An Ok of a Lock or a
  // sign-out may carry a warning (FB-24): its sentence becomes the neutral note. The account is read
  // again after ANY result (M4), so the tab never keeps showing a state the action changed.
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

  // The sentence is held the moment the sign-out answers, before the tab reads the account again: the session
  // boundary can remount this window at any point after the sign-out, and the remount must find it held.
  const signOut = async () => {
    setConfirmSignOut(false)
    await run('clear_session', async () => {
      const result = await clearSession()
      if (result.ok) holdSignOutWarning(result.value.warning?.sentence ?? null)
      return result
    }, 'Couldn’t sign out')
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

      <SettingsGroup>
        <SettingRow
          name="signout"
          tall
          label="Sign out of this Mac"
          hint="You’ll need your password to sign back in."
          control={
            <Btn disabled={busy === 'clear_session'} onClick={() => setConfirmSignOut(true)}>
              Sign out…
            </Btn>
          }
        />
      </SettingsGroup>

      <ConfirmSheet
        open={confirmSignOut}
        title="Sign out of this Mac?"
        confirmLabel="Sign out"
        danger
        onCancel={() => setConfirmSignOut(false)}
        onConfirm={() => void signOut()}
      >
        Sync stops until you sign in again.
      </ConfirmSheet>
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
function ConfirmSheet({
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

// ── Sync ────────────────────────────────────────────────────────────────────

type RepairPhase = 'idle' | 'confirming' | 'busy'

function SyncTab({ settings }: { settings: SettingsConfig }) {
  const { showToast } = useToast()
  // The Finder row is the reconciler's state, read and followed by the one shared hook (lead
  // ruling 7b): it loads, follows `finder-setup-changed` (no polling), and toasts a failed ACTION.
  // A failed SETUP is part of the state: it gates Finder, so it is the inline notice under the
  // row and never also a toast. Nothing here adds Beebeeb to Finder by hand (R5): the reconciler does.
  const finder = useFinderSetup()
  const [repair, setRepair] = useState<RepairPhase>('idle')
  const [repairFailed, setRepairFailed] = useState(false)
  const [repairResult, setRepairResult] = useState<string | null>(null)
  // Task 1882 round 2 (review I2, lead ruling): where macOS last kept Finder files that had not
  // reached the server, after a sign-out, a Repair, another reconciler removal or the app-start sweep. Rust
  // saves it in desktop.toml; this row shows it until the person dismisses it, so a tab switch,
  // closing Settings or a restart cannot lose it. Rebase re-review I2: Rust says when it saved one
  // (`kept-folder-changed`), and the row reads it again then.
  const [kept, setKept] = useState<string | null>(null)
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

  // A failed read shows no row: the folder stays saved, and the next open shows it again.
  const loadKept = useCallback(async () => {
    const result = await command<string | null>('kept_unsynced_folder')
    const note = result.ok ? preservedFilesNote({ preserved_location: result.value }) : null
    setKept(note ? note.path : null)
  }, [])

  useEffect(() => {
    void loadTree()
  }, [loadTree])

  // Rebase re-review I2: a removal the reconciler runs in the background (its Try again can start an owed one, and
  // one can finish after the reconciler's time limit) saves the folder long after this tab opened. Rust then emits
  // `kept-folder-changed`, and the row reads the folder again. The first read waits for the listener, so a folder
  // saved between the two is not missed.
  useEffect(
    () => subscribeKeptFolderChanged(() => void loadKept(), { onSubscribed: () => void loadKept() }),
    [loadKept],
  )

  // A one-off action that gates nothing: a failure is a toast (house rule), and the row stays.
  const dismissKept = async () => {
    if (kept === null) return
    const result = await command<DismissKeptFolderResult>('dismiss_kept_unsynced_folder', { path: kept })
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t dismiss that note',
        message: result.unsupported ? commandUnavailableLabel('dismiss_kept_unsynced_folder') : result.reason,
      })
      return
    }
    // 1882 r5: the row goes only if Rust cleared the folder it showed. If a newer folder was kept
    // since the row was drawn, nothing was dismissed, and the row shows the newer one.
    setKept(keptFolderAfterDismiss(result.value))
  }

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
        {kept !== null ? (
          <Note
            kind="status"
            reason={kept}
            reasonWraps
            actions={<Btn onClick={() => void dismissKept()}>Dismiss</Btn>}
          >
            {KEPT_FOLDER_ROW_SENTENCE}
          </Note>
        ) : null}
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

function AboutTab() {
  const { showToast } = useToast()
  const update = useSyncExternalStore(desktopUpdateCheck.subscribe, desktopUpdateCheck.getSnapshot, desktopUpdateCheck.getSnapshot)
  const [version, setVersion] = useState<string | null>(null)
  const [installing, setInstalling] = useState(false)
  const [exporting, setExporting] = useState(false)

  useEffect(() => {
    let cancelled = false
    void command<string>('app_version').then((result) => {
      if (!cancelled && result.ok) setVersion(result.value)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const installUpdate = async () => {
    setInstalling(true)
    const result = await command<void>('install_update')
    setInstalling(false)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Update install failed',
        message: result.unsupported ? commandUnavailableLabel('install_update') : result.reason,
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

  const row = updateRow(update, version)

  return (
    <>
      <SettingsGroup>
        <SettingRow
          name="version"
          tall
          label="Beebeeb for Mac"
          hint={row.hint}
          control={
            row.action === 'install' ? (
              <Btn primary disabled={installing} onClick={() => void installUpdate()}>
                {installing ? 'Installing…' : 'Restart to update'}
              </Btn>
            ) : (
              <Btn disabled={row.action === 'checking'} onClick={() => void desktopUpdateCheck.check()}>
                Check for updates
              </Btn>
            )
          }
        />
      </SettingsGroup>
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
    </>
  )
}

// ── The window ──────────────────────────────────────────────────────────────

export default function MacSettings({ initialTab }: { initialTab?: SettingsTab }) {
  const settings = useSettingsConfig()
  // A remount after a sign-out that left a sentence opens on the Account tab, where it is shown (accountSession.ts).
  const [tab, setTab] = useState<SettingsTab>(() => initialTab ?? (heldSignOutWarning() !== null ? 'account' : settingsTabFromLocation()))
  // And a sentence that becomes held after this window mounted brings it to the Account tab too: the boundary's
  // remount and the sign-out's answer arrive in either order.
  const signOutNote = useSyncExternalStore(subscribeSignOutWarning, heldSignOutWarning, heldSignOutWarning)
  const seenSignOutNote = useRef(signOutNote)
  useEffect(() => {
    if (signOutNote === seenSignOutNote.current) return
    seenSignOutNote.current = signOutNote
    if (signOutNote !== null) setTab('account')
  }, [signOutNote])

  // Slice 6: this window is the surface the native "Check for updates…" menu item opens on
  // macOS, so it drains the pending request itself and answers inline in the About tab's row
  // (it mounts no ManualUpdateFeedback toast). A consumed request also lands the window on
  // the About tab — that is where the progress and result are rendered; a plain open
  // consumes false and keeps the General tab.
  useEffect(() => connectNativeUpdateMenu(
    (callback) => listen('menu:check-for-updates', callback),
    async () => {
      const result = await command<boolean>('consume_menu_update_check')
      if (result.ok && result.value) setTab('about')
      return result
    },
    desktopUpdateCheck.check,
  ), [])

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const next = tabAfterKey(tab, event.key)
    if (next === null) return
    event.preventDefault()
    setTab(next)
    if (typeof document !== 'undefined') document.getElementById(`ms-tab-${next}`)?.focus()
  }

  return (
    <div className="ms-shell">
      {/* The toolbar is the window's drag handle (the macOS window is meant to use an overlay
          title bar, so the native traffic lights sit at its left). The tabs inside stay clickable. */}
      <div className="ms-toolbar" data-tauri-drag-region role="tablist" aria-label="Settings" tabIndex={-1} onKeyDown={onKeyDown}>
        {SETTINGS_TABS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            id={`ms-tab-${entry.id}`}
            className="ms-tab"
            aria-selected={tab === entry.id}
            aria-controls="ms-panel"
            tabIndex={tab === entry.id ? 0 : -1}
            onClick={() => setTab(entry.id)}
          >
            <SettingsIcon name={entry.icon} size={20} strokeWidth={1.5} />
            {entry.label}
          </button>
        ))}
      </div>
      <div className="ms-pane" id="ms-panel" role="tabpanel" aria-labelledby={`ms-tab-${tab}`} tabIndex={0}>
        {tab === 'general' ? <GeneralTab settings={settings} /> : null}
        {tab === 'account' ? <AccountTab /> : null}
        {tab === 'sync' ? <SyncTab settings={settings} /> : null}
        {tab === 'about' ? <AboutTab /> : null}
      </div>
    </div>
  )
}
