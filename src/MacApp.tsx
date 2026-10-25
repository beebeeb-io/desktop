/**
 * The Beebeeb window on a Mac (task 1683 slice 1; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7, §8, §11;
 * hi-fi `design/hifi/macos-two-surface.html` sections 3 to 5).
 *
 * One window (label `main`), one sidebar. Signed in, the sidebar holds Home, Review and Trash (slice 4: Review shows how
 * many things wait; both need the vault unlocked and show the locked card without it) and, in its footer, the account
 * and the gear. The gear, ⌘, and the menus swap the sidebar for the Settings list in System Settings style (ruling 1):
 * a back control that names where it returns to, the heading, the account row (Account), General, Sync and, after a
 * gap, About. Security joins in slice 5. The pane hosts the shipped section bodies of `MacSettings.tsx` unchanged.
 *
 * Every opener reaches the window through one request (`main:navigate`, `mainWindowModel.navigate`). While a sheet is
 * open nothing navigates (§8.2) and Rust disables View › Home, View › Back and Settings…, so ⌘1, ⌘[ and ⌘, cannot
 * unmount Settings › Account under its sheet. 1887's sign-out flow and its listeners live here, in the shell, so a
 * flow request reaches them whichever section shows; a request selects Settings › Account first, as the Settings
 * window's root did.
 *
 * Signed out, until slice 2 brings sign-in into this window, the window shows one centred card that opens today's
 * sign-in (the onboarding window, or "Sign in again" for an ended session). Settings then shows General and About.
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported here,
 * so `tests/macApp.test.tsx` can execute them with the component harness.
 */
import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  accountSubscription,
  BILLING_URL,
  command,
  commandUnavailableLabel,
  forceReauth,
  openUrl,
  popoverSnapshot,
  type CommandResult,
  type Subscription,
} from './desktopApi'
import type { PopoverSnapshot } from './popoverContract'
import { heldSignOutWarning, subscribeSignOutWarning } from './accountSession'
import { finderOpenFailedToast, useFinderSetup } from './finderSetup'
import { finderActionButtonLabel } from './finderSetupCopy'
import { KeptFolderNotes, useKeptFolders } from './keptFolderRows'
import {
  accountInitial,
  finderHint,
  finderRow,
  monoReason,
  planShortLine,
  SETTINGS_TABS,
  storageLine,
  type SettingsTab,
} from './macSettingsModel'
import { LockedPane } from './LockedPane'
import { Btn, Note, SettingsGroup, SettingsIcon } from './macSettingsParts'
import { AboutTab, AccountTab, GeneralTab, showSignOutOutcome, SyncTab, useSettingsConfig } from './MacSettings'
import {
  backLabel,
  connectMainNavigation,
  focusAfterNav,
  GEAR_ID,
  homeStatus,
  initialMainView,
  listAfterKey,
  MAIN_SECTIONS,
  mainButtonId,
  menuStateFor,
  navigate,
  sectionTitle,
  settingsButtonId,
  settingsSections,
  SnapshotReader,
  type MainNav,
  type MainView,
} from './mainWindowModel'
import { connectReviewFeed, refreshReviewFeed, reviewFeed, reviewFeedState, subscribeReviewFeed } from './reviewFeed'
import { reviewBadge } from './reviewModel'
import { ReviewPane } from './ReviewPane'
import { connectSignOutFlowEvents, connectSignOutFlowMenu, SignOutFlow, signOutFlowApi, type FlowView } from './signOutFlow'
import { TrashPane } from './TrashPane'
import { openModalCount, subscribeOpenModals, useToast } from './windows/ui'
import { connectNativeUpdateMenu, desktopUpdateCheck } from './windows/manualUpdateCheck'

/** How often the window reads the account and sync state again while it is open. */
const SNAPSHOT_REFRESH_MS = 5000

function locationSearch(): string {
  try {
    return globalThis.location?.search ?? ''
  } catch {
    return ''
  }
}

function focusById(id: string) {
  if (typeof document === 'undefined') return
  document.getElementById(id)?.focus()
}

/** The account's second line, in the sidebar footer and the Settings account row: "Vault locked" or the plan. */
function accountLine(account: PopoverSnapshot['account'], plan: Subscription | null | undefined): string | null {
  if (!account.vault_unlocked) return 'Vault locked'
  return planShortLine(plan)
}

// ── Home (§7.6, first cut: status, Finder row, storage, the kept rows) ────────

function HomePane({ snapshot, onRefresh }: { snapshot: PopoverSnapshot; onRefresh: () => void }) {
  const { showToast } = useToast()
  const finder = useFinderSetup()
  const kept = useKeptFolders()
  const [busy, setBusy] = useState<'open' | 'resume' | 'unlock' | null>(null)

  if (!snapshot.account.vault_unlocked) {
    const unlock = async () => {
      setBusy('unlock')
      const result = await command<void>('unlock_vault')
      setBusy(null)
      if (!result.ok) {
        showToast({
          variant: 'error',
          title: 'Couldn’t unlock the vault',
          message: result.unsupported ? commandUnavailableLabel('unlock_vault') : result.reason,
        })
      }
      onRefresh()
    }
    return (
      <div className="mw-content">
        <div className="mw-card mw-locked">
          <span className="mw-tile mw-tile--amber" aria-hidden="true">
            <SettingsIcon name="lock" size={16} />
          </span>
          <div className="mw-row-text">
            <div className="mw-row-title">Your vault is locked</div>
            <div className="mw-row-hint">Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.</div>
          </div>
          <Btn primary disabled={busy === 'unlock'} onClick={() => void unlock()}>
            {busy === 'unlock' ? 'Unlocking…' : 'Unlock vault'}
          </Btn>
        </div>
      </div>
    )
  }

  const status = homeStatus(snapshot, Math.floor(Date.now() / 1000))
  const row = finderRow(finder.load.status === 'loaded' ? finder.load.view : null, finder.load.status === 'unavailable', finder.refusal)
  const storage = storageLine(snapshot.storage)
  const attention = kept.kept !== null || kept.keptUnsent !== null

  const openInFinder = async () => {
    setBusy('open')
    const result = await command<void>('open_finder_location', { path: null })
    setBusy(null)
    if (!result.ok) showToast(finderOpenFailedToast())
  }

  const resume = async () => {
    setBusy('resume')
    const result = await command<void>('tray_resume_sync')
    setBusy(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t resume sync',
        message: result.unsupported ? commandUnavailableLabel('tray_resume_sync') : result.reason,
      })
    }
    onRefresh()
  }

  let statusAction = null
  if (status.action === 'resume') {
    statusAction = (
      <Btn primary disabled={busy === 'resume'} onClick={() => void resume()}>
        Resume sync
      </Btn>
    )
  } else if (status.action === 'add_storage') {
    statusAction = <Btn primary onClick={() => void openUrl(BILLING_URL)}>Add storage</Btn>
  }

  let finderControl = null
  if (row.kind === 'added') {
    finderControl = (
      <Btn primary disabled={busy === 'open'} onClick={() => void openInFinder()}>
        Open in Finder
      </Btn>
    )
  } else if (row.kind === 'adding') {
    finderControl = <Btn disabled>Adding…</Btn>
  } else if (row.kind === 'unavailable') {
    finderControl = <Btn onClick={() => void finder.retry()}>Try again</Btn>
  }

  return (
    <div className="mw-content">
      <div className="mw-card" data-home="status">
        <div className="mw-row" data-row="status">
          <span className={`mw-status-disc mw-status-disc--${status.tone}`} aria-hidden="true">
            <SettingsIcon name={status.tone === 'ok' ? 'check' : status.tone === 'alert' ? 'alert' : status.tone === 'busy' ? 'refresh' : 'info'} size={12} strokeWidth={2.2} />
          </span>
          <div className="mw-row-text">
            <div className="mw-row-title">{status.title}</div>
            {status.detail ? <div className="mw-row-hint">{status.detail}</div> : null}
            {status.reason ? <div className="ms-mono">{status.reason}</div> : null}
          </div>
          {statusAction}
        </div>
        <div className="mw-row" data-row="finder">
          <span className="mw-tile" aria-hidden="true">
            <SettingsIcon name="folder" size={16} />
          </span>
          <div className="mw-row-text">
            <div className="mw-row-title">Beebeeb in Finder</div>
            <div className="mw-row-hint">{row.kind === 'added' ? 'Your vault appears under Locations in Finder.' : finderHint(row)}</div>
          </div>
          {finderControl}
        </div>
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
      </div>

      <div className="mw-columns">
        {attention ? (
          <SettingsGroup title="Needs your attention">
            <KeptFolderNotes folders={kept} />
          </SettingsGroup>
        ) : null}
        {storage ? (
          <SettingsGroup title="Storage">
            <div className="mw-storage">
              <div className="mw-storage-line">
                <span className="mw-row-title">{storage.label}</span>
                <span className="mw-row-hint">{Math.round(storage.percent)} %</span>
              </div>
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
              <div>
                <Btn onClick={() => void openUrl(BILLING_URL)}>Add storage</Btn>
              </div>
            </div>
          </SettingsGroup>
        ) : null}
      </div>
    </div>
  )
}

// ── Signed out (until slice 2 brings sign-in into this window) ───────────────

function SignedOutCard({
  reauth,
  email,
  note,
  busy,
  onSignIn,
}: {
  reauth: boolean
  email: string | null
  note: string | null
  busy: boolean
  onSignIn: () => void
}) {
  return (
    <div className="mw-solo-body">
      {note ? (
        <div className="mw-solo-note">
          <Note kind="status">{note}</Note>
        </div>
      ) : null}
      <section className="mw-signin" aria-labelledby="mw-title">
        <div className="mw-mark" aria-hidden="true">b</div>
        <h1 id="mw-title" className="mw-signin-title">{reauth ? 'Sign in again' : 'Sign in to Beebeeb'}</h1>
        {reauth ? <p className="mw-signin-copy">Sync is paused until you sign in again. Your files stay in Finder.</p> : null}
        <Btn primary disabled={busy} onClick={onSignIn}>
          {reauth ? 'Sign in again' : 'Sign in'}
        </Btn>
        {reauth && email ? <p className="mw-signin-foot">Signed in before as {email}</p> : null}
      </section>
    </div>
  )
}

// ── The window ───────────────────────────────────────────────────────────────

export default function MacApp() {
  const settings = useSettingsConfig()
  const { showToast } = useToast()
  const showToastRef = useRef(showToast)
  showToastRef.current = showToast

  const [snapshot, setSnapshot] = useState<PopoverSnapshot | null>(null)
  const [snapshotFailed, setSnapshotFailed] = useState(false)
  const [plan, setPlan] = useState<Subscription | null | undefined>(undefined)
  const [view, setView] = useState<MainView>(() => initialMainView(locationSearch()))
  const [lastSettings, setLastSettings] = useState<SettingsTab | null>(() => (view.mode === 'settings' ? view.section : null))
  const [focusId, setFocusId] = useState<string | null>(null)
  const [signingIn, setSigningIn] = useState(false)
  const openModals = useSyncExternalStore(subscribeOpenModals, openModalCount, openModalCount)
  // Slice 4: Review's feed is one reader shared by the sidebar's badge and the Review pane. "Version history…" and
  // Trash's "Refresh" are buttons of this toolbar; the panes receive them as `historyOpen` and `refreshSignal`.
  const reviewFeedNow = useSyncExternalStore(subscribeReviewFeed, reviewFeedState, reviewFeedState)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [trashRefresh, setTrashRefresh] = useState(0)

  // Spec 2026-10-10 §5.1 and F9: the sign-out flow and its pending request live in the shell, not in the Account
  // section, which mounts only while it is selected. A flow it owned would be new at every return to it, and a
  // request it had handled would run again.
  const [flowView, setFlowView] = useState<FlowView>({ kind: 'idle' })
  const [flowOutcomes, setFlowOutcomes] = useState(0)
  const flowRef = useRef<SignOutFlow | null>(null)
  if (flowRef.current === null) {
    flowRef.current = new SignOutFlow(signOutFlowApi, setFlowView, (outcome) => {
      showSignOutOutcome(outcome, (toast) => showToastRef.current(toast))
      setFlowOutcomes((n) => n + 1)
    })
  }
  const flow = flowRef.current

  const account = snapshot?.account ?? null
  const signedIn = account?.logged_in === true
  const unlocked = account?.vault_unlocked === true
  const sheetOpen = flowView.kind !== 'idle' || openModals > 0

  // The listeners below are registered once; they read the window's current state through these.
  const viewRef = useRef(view)
  viewRef.current = view
  const ctxRef = useRef({ signedIn, lastSettings, sheetOpen })
  ctxRef.current = { signedIn, lastSettings, sheetOpen }
  // A request can arrive before the first account read (a window Rust built for it pulls it on mount): it waits for
  // that read, because which sections exist depends on the session. Each waits as the step it is, in order.
  const loadedRef = useRef(false)
  loadedRef.current = account !== null
  const waitingRef = useRef<Array<() => void>>([])
  const whenLoaded = (step: () => void) => {
    if (loadedRef.current) step()
    else waitingRef.current.push(step)
  }
  useEffect(() => {
    if (account === null || waitingRef.current.length === 0) return
    const steps = waitingRef.current
    waitingRef.current = []
    for (const step of steps) step()
  }, [account])

  /** The one way the window moves. `force` is the sign-out flow's request only: it selects Account even under a sheet. */
  const go = (nav: MainNav, force = false) => whenLoaded(() => move(nav, force))
  const move = (nav: MainNav, force: boolean) => {
    const previous = viewRef.current
    const ctx = force ? { ...ctxRef.current, sheetOpen: false } : ctxRef.current
    const next = navigate(previous, nav, ctx)
    if (next === previous) return
    viewRef.current = next
    setView(next)
    if (next.mode === 'settings') setLastSettings(next.section)
    setFocusId(focusAfterNav(previous, next, nav))
  }

  // The account and sync state: on open, every few seconds, and after each sign-out outcome. The reader keeps one read
  // in flight and never throws one away because the timer fired (a read can take 8 s, the tick is 5 s); an outcome's
  // read wins over the reads started before it (follow-up to PR #134, auto-review thread 1).
  const readerRef = useRef<SnapshotReader<CommandResult<PopoverSnapshot>> | null>(null)
  if (readerRef.current === null) {
    readerRef.current = new SnapshotReader(
      () => popoverSnapshot(1),
      (result) => {
        if (result.ok) {
          setSnapshot(result.value)
          setSnapshotFailed(false)
        } else {
          setSnapshotFailed(true)
        }
      },
    )
  }
  const reader = readerRef.current
  useEffect(() => {
    reader.resume()
    reader.tick()
    const id = window.setInterval(() => reader.tick(), SNAPSHOT_REFRESH_MS)
    return () => {
      window.clearInterval(id)
      reader.stop()
    }
  }, [reader])
  useEffect(() => {
    if (flowOutcomes > 0) reader.refresh()
  }, [reader, flowOutcomes])

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

  // A sign-out's sentence that becomes held while this window is mounted and signed in brings it to Settings ›
  // Account, where it is shown (accountSession.ts). After the boundary's remount, signed out, the card shows it.
  const signOutNote = useSyncExternalStore(subscribeSignOutWarning, heldSignOutWarning, heldSignOutWarning)
  const seenSignOutNote = useRef(signOutNote)
  useEffect(() => {
    if (signOutNote === seenSignOutNote.current) return
    seenSignOutNote.current = signOutNote
    if (signOutNote !== null && ctxRef.current.signedIn) go({ to: 'settings', section: 'account' }, true)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `go` reads the refs above; the effect follows the note only.
  }, [signOutNote])

  // Every opener: the gear and the menus in Rust, the popover's gear menu from slice 3.
  useEffect(
    () => connectMainNavigation(listen, () => command<unknown>('consume_main_navigation'), (nav) => go(nav)),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- registered once; `go` reads the refs above.
    [],
  )

  // The native "Check for Updates…": this window drains the pending request and answers inline in Settings › About's
  // row (it mounts no ManualUpdateFeedback toast). A consumed request lands on About; a plain open consumes false.
  useEffect(
    () =>
      connectNativeUpdateMenu(
        (callback) => listen('menu:check-for-updates', callback),
        async () => {
          const result = await command<boolean>('consume_menu_update_check')
          if (result.ok && result.value) go({ to: 'settings', section: 'about' })
          return result
        },
        desktopUpdateCheck.check,
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- registered once; `go` reads the refs above.
    [],
  )

  useEffect(() => connectSignOutFlowEvents(listen, flow), [flow])
  useEffect(
    () =>
      connectSignOutFlowMenu(listen, () => command<boolean>('consume_sign_out_flow_request'), () =>
        whenLoaded(() => {
          move({ to: 'settings', section: 'account' }, true)
          void flow.request()
        }),
      ),
    [flow],
  )

  // The Review feed is read only while there is a session and an unlocked vault: a locked vault, or another account,
  // never sees the last one's rows (the feed drops them when the last reader stops).
  useEffect(
    () => (signedIn && unlocked ? connectReviewFeed(reviewFeed, listen) : undefined),
    [signedIn, unlocked],
  )

  // The history sheet belongs to Review: leaving Review (the sign-out flow can force Settings › Account) closes it, and so
  // does a vault that locks (Review then shows the locked card; the sheet must not be waiting behind it when the vault
  // unlocks again).
  const inReview = view.mode === 'main' && view.section === 'review'
  useEffect(() => {
    if (!(inReview && unlocked)) setHistoryOpen(false)
  }, [inReview, unlocked])

  // The rows are read only while Review shows them; the badge's count is read all the time.
  useEffect(() => (signedIn && unlocked && inReview ? reviewFeed.watchList() : undefined), [signedIn, unlocked, inReview])

  // §8.2 and F15: Rust enables View › Back only while Settings shows, and disables the navigation items and
  // Settings… while a sheet is open.
  const settingsShowing = view.mode === 'settings'
  useEffect(() => {
    void command<void>('main_window_state', menuStateFor(viewRef.current, sheetOpen))
  }, [settingsShowing, sheetOpen])

  // §11: focus follows a navigation (the selected section, or the gear after back).
  useEffect(() => {
    if (focusId === null) return
    focusById(focusId)
    setFocusId(null)
  }, [focusId])

  if (snapshot === null || account === null) {
    return (
      <div className="mw-window mw-window--solo">
        <div className="mw-toolbar mw-toolbar--solo" data-tauri-drag-region="" />
        <div className="mw-solo-body">
          {snapshotFailed ? (
            <Note
              kind="alert"
              surface="main-load"
              title="Couldn’t load your account"
              actions={<Btn onClick={() => reader.refresh()}>Try again</Btn>}
            />
          ) : (
            <div className="ms-loading" role="status">Loading…</div>
          )}
        </div>
      </div>
    )
  }

  const base: 'home' | 'signin' | 'reauth' = !account.logged_in ? 'signin' : account.auth_expired ? 'reauth' : 'home'
  const line = accountLine(account, plan)

  const signIn = async () => {
    setSigningIn(true)
    if (base === 'reauth') {
      const result = await forceReauth()
      setSigningIn(false)
      if (!result.ok) showToast({ variant: 'error', title: 'Couldn’t start sign-in again', message: result.reason })
      else if (result.value.warning) showToast({ variant: 'info', message: result.value.warning.sentence })
      return
    }
    const opened = await command<void>('open_onboarding_window')
    setSigningIn(false)
    if (!opened.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t open sign-in',
        message: opened.unsupported ? commandUnavailableLabel('open_onboarding_window') : opened.reason,
      })
    }
  }

  if (view.mode === 'main' && base !== 'home') {
    return (
      <div className="mw-window mw-window--solo">
        <div className="mw-toolbar mw-toolbar--solo" data-tauri-drag-region="" />
        <main className="mw-solo-main" aria-labelledby="mw-title">
          <SignedOutCard reauth={base === 'reauth'} email={account.email} note={signOutNote} busy={signingIn} onSignIn={() => void signIn()} />
        </main>
      </div>
    )
  }

  const onMainListKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (view.mode !== 'main') return
    const ids = MAIN_SECTIONS.map((s) => s.id)
    const next = listAfterKey(ids, view.section, event.key)
    if (next === null) return
    event.preventDefault()
    go({ to: next })
    focusById(mainButtonId(next))
  }

  const sections = settingsSections(signedIn)
  const onSettingsListKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (view.mode !== 'settings') return
    const next = listAfterKey(sections, view.section, event.key)
    if (next === null) return
    event.preventDefault()
    go({ to: 'settings', section: next })
  }

  const sectionLabel = (id: SettingsTab) => SETTINGS_TABS.find((tab) => tab.id === id)?.label ?? id
  const sectionIcon = (id: SettingsTab) => SETTINGS_TABS.find((tab) => tab.id === id)?.icon ?? 'info'

  const sidebar =
    view.mode === 'settings' ? (
      <nav className="mw-sidebar mw-sidebar--settings" aria-label="Settings">
        <div className="mw-drag-band" data-tauri-drag-region="" />
        <button
          type="button"
          className="mw-back"
          aria-label={`Back to ${backLabel(view.returnTo, base === 'home')}`}
          onClick={() => go({ to: 'back' })}
        >
          <SettingsIcon name="back" size={14} strokeWidth={1.8} />
          {backLabel(view.returnTo, base === 'home')}
        </button>
        <h2 className="mw-settings-heading">Settings</h2>
        {/* One Tab stop (roving tabindex): each item takes the list's keys (§11). */}
        <div className="mw-list">
          {sections.map((id) =>
            id === 'account' ? (
              <button
                key={id}
                type="button"
                id={settingsButtonId(id)}
                className="mw-account-row"
                aria-current={view.section === id ? 'page' : undefined}
                aria-label={['Account', account.email, line].filter(Boolean).join(', ')}
                tabIndex={view.section === id ? 0 : -1}
                onKeyDown={onSettingsListKey}
                onClick={() => go({ to: 'settings', section: id })}
              >
                <span className="mw-avatar mw-avatar--large" aria-hidden="true">{accountInitial(account.email)}</span>
                <span className="mw-account-text">
                  <span className="mw-email">{account.email ?? 'Signed in'}</span>
                  {line ? <span className="mw-plan">{line}</span> : null}
                </span>
              </button>
            ) : (
              <div key={id} className={id === 'about' ? 'mw-item mw-item--apart' : 'mw-item'}>
                <button
                  type="button"
                  id={settingsButtonId(id)}
                  className="mw-nav"
                  aria-current={view.section === id ? 'page' : undefined}
                  tabIndex={view.section === id ? 0 : -1}
                  onKeyDown={onSettingsListKey}
                  onClick={() => go({ to: 'settings', section: id })}
                >
                  <span className="mw-nav-tile" aria-hidden="true">
                    <SettingsIcon name={id === 'general' ? 'gear' : sectionIcon(id)} size={13} strokeWidth={1.7} />
                  </span>
                  {sectionLabel(id)}
                </button>
              </div>
            ),
          )}
        </div>
      </nav>
    ) : (
      <nav className="mw-sidebar" aria-label="Beebeeb">
        {/* The band beside the traffic lights drags the window, as the toolbar does (review item 7). */}
        <div className="mw-drag-band" data-tauri-drag-region="" />
        <div className="mw-list">
          {MAIN_SECTIONS.map((section) => {
            // Review's number: the feed's COUNT (all that wait, not the rows it carries), only once the count has been
            // read (0 until then: no placeholder, no guess) and the vault is unlocked.
            const badge = section.id === 'review' ? reviewBadge(reviewFeedNow.count, unlocked) : null
            return (
              <button
                key={section.id}
                type="button"
                id={mainButtonId(section.id)}
                className="mw-nav"
                aria-current={view.section === section.id ? 'page' : undefined}
                aria-label={badge === null ? undefined : `${section.label}, ${badge} ${badge === 1 ? 'needs' : 'need'} attention`}
                tabIndex={view.section === section.id ? 0 : -1}
                onKeyDown={onMainListKey}
                onClick={() => go({ to: section.id })}
              >
                <SettingsIcon name={section.icon} size={16} />
                {section.label}
                {badge === null ? null : <span className="mw-badge" aria-hidden="true">{badge}</span>}
              </button>
            )
          })}
        </div>
        <div className="mw-foot">
          <span className="mw-avatar" aria-hidden="true">{accountInitial(account.email)}</span>
          <span className="mw-account-text">
            <span className="mw-email">{account.email ?? 'Signed in'}</span>
            {line ? <span className="mw-plan">{line}</span> : null}
          </span>
          <button type="button" id={GEAR_ID} className="mw-gear" aria-label="Settings" onClick={() => go({ to: 'settings', section: null })}>
            <SettingsIcon name="gear" size={18} strokeWidth={1.5} />
          </button>
        </div>
      </nav>
    )

  let pane = null
  if (view.mode === 'main') {
    if (view.section === 'home') {
      pane = <HomePane snapshot={snapshot} onRefresh={() => reader.refresh()} />
    } else if (!unlocked) {
      // §7.6: Review and Trash need the key and show Home's locked card.
      pane = <LockedPane onRefresh={() => reader.refresh()} />
    } else if (view.section === 'review') {
      pane = <ReviewPane feed={reviewFeedNow} historyOpen={historyOpen} onHistoryClose={() => setHistoryOpen(false)} onChanged={refreshReviewFeed} />
    } else {
      pane = <TrashPane refreshSignal={trashRefresh} />
    }
  } else {
    pane = (
      <div className="ms-pane mw-pane" id="mw-pane">
        {view.section === 'general' ? <GeneralTab settings={settings} /> : null}
        {view.section === 'account' ? <AccountTab flow={flow} flowView={flowView} flowOutcomes={flowOutcomes} /> : null}
        {view.section === 'sync' ? <SyncTab settings={settings} /> : null}
        {view.section === 'about' ? <AboutTab /> : null}
      </div>
    )
  }

  return (
    <div className="mw-window">
      {sidebar}
      <main className="mw-main" aria-labelledby="mw-title">
        {/* The toolbar is the window's drag handle, its title and chip included ("deep": a button in it still clicks).
            The overlay title bar puts the traffic lights over the sidebar. */}
        <div className="mw-toolbar" data-tauri-drag-region="deep">
          <h1 id="mw-title" className="mw-title">{sectionTitle(view)}</h1>
          {/* The hi-fi's Home toolbar (section 3): the encryption state, in amber, while the vault is unlocked. Review
              and Trash draw their own buttons there instead (slice 4). */}
          {view.mode === 'main' && view.section === 'home' && unlocked ? (
            <span className="mw-chip">
              <SettingsIcon name="lock" size={13} />
              End-to-end encrypted
            </span>
          ) : null}
          {/* Hi-fi section 3: the section's own buttons sit at the right of the toolbar, neutral (the amber is for the
              one primary action of a view, and neither of these is it). */}
          {view.mode === 'main' && unlocked && view.section === 'review' ? (
            <div className="mw-toolbar-actions">
              <Btn onClick={() => setHistoryOpen(true)}>
                <SettingsIcon name="history" size={14} />
                Version history…
              </Btn>
            </div>
          ) : null}
          {view.mode === 'main' && unlocked && view.section === 'trash' ? (
            <div className="mw-toolbar-actions">
              <Btn onClick={() => setTrashRefresh((n) => n + 1)}>Refresh</Btn>
            </div>
          ) : null}
        </div>
        {pane}
      </main>
    </div>
  )
}
