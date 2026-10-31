/**
 * The Beebeeb window on a Mac (task 1683 slice 1; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7, §8, §11;
 * hi-fi `design/hifi/macos-two-surface.html` sections 3 to 5).
 *
 * One window (label `main`), one sidebar. Signed in, the sidebar holds Home, Review and Trash (slice 4: Review shows how
 * many things wait; both need the vault unlocked and show the locked card without it) and, in its footer, the account
 * and the gear. The gear, ⌘, and the menus swap the sidebar for the Settings list in System Settings style (ruling 1):
 * a back control that names where it returns to, the heading, the account row (Account), General, Sync, Security and,
 * after a gap, About. The pane hosts the section bodies of `MacSettings.tsx` and `MacSecurity.tsx`.
 *
 * Every opener reaches the window through one request (`main:navigate`, `mainWindowModel.navigate`). While a sheet is
 * open nothing navigates (§8.2) and Rust disables View › Home, View › Back and Settings…, so ⌘1, ⌘[ and ⌘, cannot
 * unmount Settings › Account under its sheet. 1887's sign-out flow and its listeners live here, in the shell, so a
 * flow request reaches them whichever section shows; a request selects Settings › Account first, as the Settings
 * window's root did.
 *
 * Signed out, the window shows one centred card at a time and no sidebar (slice 2, spec §7.4, `macSignIn.tsx`): sign in
 * (browser first, password second), Sign in again for an ended session, the account switch, set up this Mac, and the
 * Finder status, after which it moves on to Home. Which card follows from the session and from the sign-in FLOW
 * (`signInModel.ts`), which lives in a module-level store because `AccountSessionBoundary` remounts this window on every
 * session change and a sign-in is one. Settings then shows General and About.
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
  popoverSnapshot,
  vaultUnlockRoute,
  type CommandResult,
  type Subscription,
  type VaultUnlockRoute,
  type VersionConflictEntry,
} from './desktopApi'
import type { PopoverSnapshot } from './popoverContract'
import { accountSessionRevision, heldSignOutWarning, subscribeAccountSession, subscribeSignOutWarning } from './accountSession'
import { LOAD_FAILED_TITLE, SET_UP_COPY, SIGNED_IN_WINDOW_TITLE, USE_RECOVERY_PHRASE } from './accountViewCopy'
import { ConflictSheet } from './ConflictSheet'
import { useFileOverview } from './fileOverview'
import { finderOpenFailedToast, useFinderSetup } from './finderSetup'
import { finderActionButtonLabel } from './finderSetupCopy'
import { focusById } from './focusById'
import { HomeActivity, HomeAttention, HomeCounts } from './HomeSections'
import { activityRowsForHome, attentionForHome, HOME_ACTIVITY_ROWS, homeCounts } from './homeModel'
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
import { moveFlow, SignInWindow } from './macSignIn'
import { MenuBarHint } from './MenuBarHint'
import { AboutTab, AccountTab, GeneralTab, showSignOutOutcome, SyncTab, useSettingsConfig } from './MacSettings'
import { SecurityTab } from './MacSecurity'
import {
  backLabel,
  coerceSettingsView,
  connectMainNavigation,
  focusAfterNav,
  GEAR_ID,
  homeStatus,
  heldNoteGoesToSettings,
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
import { makeReviewActions } from './reviewActions'
import { connectReviewFeed, refreshReviewFeed, reviewFeed, reviewFeedState, subscribeReviewFeed, type ReviewFeedState } from './reviewFeed'
import { reviewBadge } from './reviewModel'
import { ReviewPane } from './ReviewPane'
import { getSignInFlow, setSignInFlow, subscribeSignInFlow } from './signInFlowStore'
import { canLeaveSignInAgain, flowForNav, flowForPendingSetup, IDLE_FLOW, isNoVaultKey, normalizeFlow, setupCheckWanted, windowScreen } from './signInModel'
import { connectSignOutFlowEvents, connectSignOutFlowMenu, SignOutFlow, signOutFlowApi, type FlowView } from './signOutFlow'
import { systemLocale } from './storageFormat'
import { TrashPane } from './TrashPane'
import { setMainWindowTitle } from './windowTitle'
import { openModalCount, subscribeOpenModals, useToast } from './windows/ui'
import { connectNativeUpdateMenu, desktopUpdateCheck } from './windows/manualUpdateCheck'

/** How long after a read finishes the window reads the account and sync state again while it is open. */
const SNAPSHOT_REFRESH_MS = 5000
/** A snapshot read still unanswered after this long is given up (task 2179): it fails, and the next one starts. */
const SNAPSHOT_WATCHDOG_MS = 15_000

/** What the reader applies for a read that threw, rejected or never answered. */
function snapshotReadFailure(cause: 'threw' | 'rejected' | 'timed out', error?: unknown): CommandResult<PopoverSnapshot> {
  const reason = cause === 'timed out' ? 'Beebeeb didn’t answer in time.' : error instanceof Error ? error.message : String(error ?? cause)
  return { ok: false, reason, unsupported: false }
}

function locationSearch(): string {
  try {
    return globalThis.location?.search ?? ''
  } catch {
    return ''
  }
}

/** The account's second line, in the sidebar footer and the Settings account row: "Vault locked" or the plan. */
function accountLine(account: PopoverSnapshot['account'], plan: Subscription | null | undefined): string | null {
  if (!account.vault_unlocked) return 'Vault locked'
  return planShortLine(plan)
}

// ── Home (§7.6: status, the Finder row and the counts line, what needs attention, storage, recent activity) ──

type BusyKey = 'open' | 'resume' | 'unlock'
/** `running`: the command is under way. `waiting`: it answered, but the window has not yet shown a snapshot read after
 * it (its own refresh failed, timed out or was superseded), so the card on screen may still be the one from before. */
type BusyState = 'running' | 'waiting'

function HomePane({
  snapshot,
  onRefresh,
  feed,
  onFeedChanged,
  onOpenReview,
}: {
  snapshot: PopoverSnapshot
  onRefresh: () => Promise<boolean>
  /** Review's feed (`reviewFeed.ts`): "Needs your attention" shows its top three. */
  feed: ReviewFeedState
  /** A conflict was decided here: read the feed again so the row leaves. */
  onFeedChanged: () => void
  onOpenReview: () => void
}) {
  const { showToast } = useToast()
  const finder = useFinderSetup()
  const kept = useKeptFolders()
  const overview = useFileOverview(snapshot.account.vault_unlocked)
  const [deciding, setDeciding] = useState<VersionConflictEntry | null>(null)
  // One busy state per button (#139 review P3-5): Open in Finder finishing never re-enables a Resume still under way.
  const [busy, setBusy] = useState<Partial<Record<BusyKey, BusyState>>>({})
  const mark = (key: BusyKey, state: BusyState | null) =>
    setBusy((current) => {
      const next = { ...current }
      if (state) next[key] = state
      else delete next[key]
      return next
    })
  // Review P2-3 b: the Keychain holds a token and no vault key, so the unlock has nothing to read. The sentence is the
  // approved Set up this Mac one, and the card offers the recovery phrase, which is what helps.
  const [noKey, setNoKey] = useState(false)
  // Every snapshot the window applies after an action is from after it (only the current epoch's reads apply), so it
  // ends the wait of a button whose own refresh did not land (#139 review P2-2).
  useEffect(() => {
    setBusy((current) => (Object.values(current).includes('waiting') ? Object.fromEntries(Object.entries(current).filter(([, state]) => state !== 'waiting')) : current))
  }, [snapshot])

  if (!snapshot.account.vault_unlocked) {
    const unlock = async () => {
      mark('unlock', 'running')
      setNoKey(false)
      // Task 1747 Part 2: the Keychain is asked at the click, not trusted from when the window opened. A Mac with no key anywhere
      // goes to Set up this Mac (the Keychain unlock would have nothing to read); a key that appeared since is honoured too.
      const routed = await vaultUnlockRoute()
      if (routed.ok && routed.value === 'recovery_phrase') {
        mark('unlock', null)
        moveFlow({ ...IDLE_FLOW, step: 'unlock' }, setSignInFlow)
        return
      }
      // A build without the command cannot tell; any other failure is the unlock failing, in its own words and surface.
      if (!routed.ok && !routed.unsupported) {
        mark('unlock', null)
        showToast({ variant: 'error', title: 'Couldn’t unlock the vault', message: routed.reason })
        onRefresh()
        return
      }
      const result = await command<void>('unlock_vault')
      if (!result.ok) {
        if (!result.unsupported && isNoVaultKey(result.reason)) setNoKey(true)
        else {
          showToast({
            variant: 'error',
            title: 'Couldn’t unlock the vault',
            message: result.unsupported ? commandUnavailableLabel('unlock_vault') : result.reason,
          })
        }
      }
      // Busy until the window shows what the unlock did (task 2179; #139 review P2-2): a stale locked card never offers
      // the button again, also when the refresh after it did not land.
      const shown = await onRefresh()
      mark('unlock', shown ? null : 'waiting')
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
          <Btn primary disabled={busy.unlock !== undefined} onClick={() => void unlock()}>
            {busy.unlock !== undefined ? 'Unlocking…' : 'Unlock vault'}
          </Btn>
        </div>
        {noKey ? (
          <Note
            kind="status"
            actions={<Btn onClick={() => moveFlow({ ...IDLE_FLOW, step: 'unlock' }, setSignInFlow)}>{USE_RECOVERY_PHRASE}</Btn>}
          >
            {SET_UP_COPY}
          </Note>
        ) : null}
      </div>
    )
  }

  const status = homeStatus(snapshot, Math.floor(Date.now() / 1000))
  const row = finderRow(finder.load.status === 'loaded' ? finder.load.view : null, finder.load.status === 'unavailable', finder.refusal)
  const storage = storageLine(snapshot.storage)
  const keptRows = kept.kept !== null || kept.keptUnsent !== null
  const now = new Date()
  const counts = homeCounts(overview, systemLocale())
  const attention = attentionForHome(feed)
  const activity = activityRowsForHome(snapshot.activity, now)
  // What a Review row's button does is one function for Review and Home (`reviewActions.ts`).
  const act = makeReviewActions({
    showToast, onDecide: setDeciding, forceReauth, onChanged: onFeedChanged,
    openUrl: (url) => command<void>('plugin:opener|open_url', { url }),
    dismiss: (entryId) => command<void>('dismiss_review_entry', { entryId }),
  })
  const openBilling = async () => {
    const result = await command<void>('plugin:opener|open_url', { url: BILLING_URL })
    if (!result.ok) showToast({ variant: 'error', title: 'Couldn’t open your browser', message: result.reason })
  }

  const openInFinder = async () => {
    mark('open', 'running')
    const result = await command<void>('open_finder_location', { path: null })
    mark('open', null)
    if (!result.ok) showToast(finderOpenFailedToast())
  }

  const resume = async () => {
    mark('resume', 'running')
    const result = await command<void>('tray_resume_sync')
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t resume sync',
        message: result.unsupported ? commandUnavailableLabel('tray_resume_sync') : result.reason,
      })
    }
    // Busy until the window shows what the resume did (task 2179; #139 review P2-2).
    const shown = await onRefresh()
    mark('resume', shown ? null : 'waiting')
  }

  let statusAction = null
  if (status.action === 'resume') {
    statusAction = (
      <Btn primary disabled={busy.resume !== undefined} onClick={() => void resume()}>
        Resume sync
      </Btn>
    )
  } else if (status.action === 'add_storage') {
    statusAction = <Btn primary onClick={() => void openBilling()}>Add storage</Btn>
  }

  let finderControl = null
  if (row.kind === 'added') {
    finderControl = (
      <Btn primary disabled={busy.open !== undefined} onClick={() => void openInFinder()}>
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
      <MenuBarHint />
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
        <HomeCounts parts={counts} />
      </div>

      <div className="mw-columns">
        <HomeAttention entries={attention.entries} more={attention.more} now={now} onAction={act} onOpenReview={onOpenReview}>
          {keptRows ? <KeptFolderNotes folders={kept} /> : null}
        </HomeAttention>
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
                <Btn onClick={() => void openBilling()}>Add storage</Btn>
              </div>
            </div>
          </SettingsGroup>
        ) : null}
      </div>

      <HomeActivity rows={activity} />

      {deciding ? <ConflictSheet entry={deciding} onClose={() => setDeciding(null)} onResolved={onFeedChanged} /> : null}
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
  const [rawView, setView] = useState<MainView>(() => initialMainView(locationSearch()))
  const [lastSettings, setLastSettings] = useState<SettingsTab | null>(() => (rawView.mode === 'settings' ? rawView.section : null))
  const [focusId, setFocusId] = useState<string | null>(null)
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

  // The sign-in flow (account switch, set up this Mac, Finder status, settling) is kept outside React: see the header.
  const signInFlow = useSyncExternalStore(subscribeSignInFlow, getSignInFlow, getSignInFlow)
  const account = snapshot?.account ?? null
  const signedIn = account?.logged_in === true
  const unlocked = account?.vault_unlocked === true
  // The flow is checked ONCE, against the first snapshot this window reads (`normalizeFlow`): a step of an account that
  // left is dropped, so the window shown again after a sign-out starts at sign-in. Never against a later snapshot, which
  // may predate the step a sign-in has just set. The card to draw, or Home, follows from that.
  const flowChecked = useRef(false)
  const flowNow = account !== null && !flowChecked.current ? normalizeFlow(account, signInFlow) : signInFlow
  // Review P2-3: a window reopened while a password sign-in was waiting for the recovery phrase (⌘W destroyed the window
  // and its flow) asks Rust once; a pending step resumes at Set up this Mac. Until Rust answers it says “Loading…”, so
  // Home's locked card is not drawn for a moment and then replaced.
  const [setupAsk, setSetupAsk] = useState<'idle' | 'asking' | 'done'>('idle')
  const askingForSetup = account !== null && setupAsk !== 'done' && setupCheckWanted(account, flowNow)
  // Task 1747 Part 2 (plan D4): the Keychain's route, asked with the same one question and for the same window: one that OPENS
  // signed in and locked. `windowScreen` settles while it is out and, with no key anywhere, shows Set up this Mac by state.
  const [route, setRoute] = useState<VaultUnlockRoute>('keychain')
  const screen = account === null ? 'home' : windowScreen(account, flowNow, askingForSetup ? 'unknown' : route)
  const sheetOpen = flowView.kind !== 'idle' || openModals > 0
  // Signed out, only General and About exist (§7.8): a session that ends under another section moves the window to General.
  // Not under a sheet: nothing navigates under one (§8.2), and the Repair sheet lives in Sync. The move comes when it closes.
  const view = sheetOpen ? rawView : coerceSettingsView(rawView, signedIn)

  // The listeners below are registered once; they read the window's current state through these.
  const viewRef = useRef(view)
  viewRef.current = view
  const ctxRef = useRef({ signedIn, lastSettings, sheetOpen })
  ctxRef.current = { signedIn, lastSettings, sheetOpen }
  // Whether a sign-in card is drawn right now (set below, once `screen` and `view` are known): a held sign-out sentence
  // is shown on the card, not by leaving it for Settings.
  const showingCardRef = useRef(false)
  // A “Sign in again” request that arrived under a sheet, applied when the sheet closes (delta review P3-2).
  const reauthHeld = useRef<MainNav | null>(null)
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

  // The account and sync state: on open, a few seconds after each read, and after each outcome. The reader keeps one
  // read in flight and never throws one away because the timer fired; an outcome's read wins over the reads started
  // before it (follow-up to PR #134, auto-review thread 1; task 2179).
  const readerRef = useRef<SnapshotReader<CommandResult<PopoverSnapshot>> | null>(null)
  // Set when a finished sign-in asks for a fresh read; the first read that lands after that ends the settling.
  const awaitingSettle = useRef(false)
  if (readerRef.current === null) {
    readerRef.current = new SnapshotReader(
      () => popoverSnapshot(HOME_ACTIVITY_ROWS),
      (result) => {
        if (result.ok) {
          setSnapshot(result.value)
          setSnapshotFailed(false)
          if (awaitingSettle.current) {
            awaitingSettle.current = false
            if (getSignInFlow().step === 'settling') setSignInFlow(IDLE_FLOW)
          }
        } else {
          setSnapshotFailed(true)
        }
      },
      { failure: snapshotReadFailure, watchdogMs: SNAPSHOT_WATCHDOG_MS },
    )
  }
  const reader = readerRef.current
  useEffect(() => {
    reader.start(SNAPSHOT_REFRESH_MS)
    return () => reader.stop()
  }, [reader])
  useEffect(() => {
    if (flowOutcomes > 0) void reader.refresh()
  }, [reader, flowOutcomes])

  // A sign-in that finished (the same account signed in again in place) must not draw the old "Sign in again" card
  // while the next read catches up: ask for that read now, and show only "Loading…" until it lands.
  useEffect(() => {
    if (signInFlow.step !== 'settling') return
    awaitingSettle.current = true
    reader.refresh()
  }, [reader, signInFlow.step])

  // The question is for a window that OPENS locked (delta review P3-3): a Lock can only follow keys that arrived, and keys
  // arriving forget the step in Rust, so after a Lock nothing is owed and asking would only paint "Loading…" for a round
  // trip. A window whose first read is not that case is done with it for good.
  useEffect(() => {
    if (account === null || setupAsk !== 'idle') return
    if (!setupCheckWanted(account, flowNow)) setSetupAsk('done')
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the first read decides; `flowNow` is that read's.
  }, [account, setupAsk])
  useEffect(() => {
    if (!askingForSetup || setupAsk !== 'idle') return
    setSetupAsk('asking')
    // Both questions at once: a step Rust remembers wins (it says why the phrase is asked), and the route covers the app having
    // been quit, which forgets that step. A route that cannot be read is the Keychain unlock, never a window stuck settling.
    void Promise.all([command<unknown>('pending_setup'), vaultUnlockRoute()]).then(([pending, routed]) => {
      const resumed = pending.ok ? flowForPendingSetup(pending.value) : null
      // A step set while Rust was answering is the person's: never replaced by a resumed one.
      if (resumed !== null && getSignInFlow().step === null) setSignInFlow(resumed)
      setRoute(routed.ok ? routed.value : 'keychain')
      setSetupAsk('done')
    })
  }, [askingForSetup, setupAsk])
  // The route is the Mac as it was when the window opened. Once the keys have arrived (the phrase restored them, or the
  // sign-in brought them: Finder is next) it is spent, so a later Lock shows Home's locked card, not Set up this Mac again.
  useEffect(() => {
    if (unlocked || signInFlow.step === 'finder') setRoute('keychain')
  }, [unlocked, signInFlow.step])

  // The one check above, kept: the store follows what the first render already showed.
  useEffect(() => {
    if (account === null || flowChecked.current) return
    flowChecked.current = true
    if (flowNow !== signInFlow) setSignInFlow(flowNow)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once, for the first snapshot; later flows are not judged.
  }, [account])

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
    if (signOutNote !== null && heldNoteGoesToSettings({ signedIn: ctxRef.current.signedIn, showingCard: showingCardRef.current })) {
      go({ to: 'settings', section: 'account' }, true)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `go` reads the refs above; the effect follows the note only.
  }, [signOutNote])

  // Every opener: the gear and the menus in Rust, the popover's gear menu from slice 3.
  useEffect(
    () =>
      connectMainNavigation(listen, () => command<unknown>('consume_main_navigation'), (nav) => {
        // “Sign in again” is a step of the sign-in flow, not a place (review P3-1): it waits for the first account read,
        // which says whether there is an account to sign in again to, and, under a sheet, for the sheet to close (§8.2,
        // delta review P3-2; `reauthHeld`).
        whenLoaded(() => {
          const { flow: next, held } = flowForNav(nav, { signedIn: ctxRef.current.signedIn, sheetOpen: ctxRef.current.sheetOpen }, getSignInFlow())
          if (held) reauthHeld.current = nav
          else if (next !== getSignInFlow()) setSignInFlow(next)
        })
        go(nav)
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- registered once; `go` reads the refs above.
    [],
  )

  // A “Sign in again” request that waited for a sheet is applied when the sheet closes, once (delta review P3-2).
  useEffect(() => {
    if (sheetOpen || !reauthHeld.current) return
    const nav = reauthHeld.current
    reauthHeld.current = null
    const { flow: next } = flowForNav(nav, { signedIn: ctxRef.current.signedIn, sheetOpen: false }, getSignInFlow())
    if (next !== getSignInFlow()) setSignInFlow(next)
  }, [sheetOpen])

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
  // never sees the last one's rows (the feed drops them when the last reader stops). The session's revision is part of the
  // lifetime: a sign-out and a sign-in as the same account between two reads change neither `signedIn` nor `unlocked`, and
  // the feed must still be a new one (review of #142, round 2).
  const sessionRevision = useSyncExternalStore(subscribeAccountSession, accountSessionRevision, accountSessionRevision)
  useEffect(
    () => (signedIn && unlocked ? connectReviewFeed(reviewFeed, listen) : undefined),
    [signedIn, unlocked, sessionRevision],
  )

  // The history sheet belongs to Review: leaving Review (the sign-out flow can force Settings › Account) closes it, and so
  // does a vault that locks (Review then shows the locked card; the sheet must not be waiting behind it when the vault
  // unlocks again).
  const inReview = view.mode === 'main' && view.section === 'review'
  useEffect(() => {
    if (!(inReview && unlocked)) setHistoryOpen(false)
  }, [inReview, unlocked])

  // The rows are read only while something shows them (Review, or Home's "Needs your attention"); the badge's count is
  // read all the time.
  const inHome = view.mode === 'main' && view.section === 'home'
  useEffect(
    () => (signedIn && unlocked && (inReview || inHome) ? reviewFeed.watchList() : undefined),
    [signedIn, unlocked, inReview, inHome, sessionRevision],
  )

  // §8.2 and F15: Rust enables View › Back only while Settings shows, and disables the navigation items and
  // Settings… while a sheet is open.
  const settingsShowing = view.mode === 'settings'
  useEffect(() => {
    void command<void>('main_window_state', menuStateFor(viewRef.current, sheetOpen))
  }, [settingsShowing, sheetOpen])

  // Spec C C-W8 and spec §10.2 row 36: signed in (or in Settings) the window is "Beebeeb"; a sign-in card sets its own
  // title, the heading it shows.
  const loaded = account !== null
  const showingCard = loaded && view.mode === 'main' && screen !== 'home'
  showingCardRef.current = showingCard
  useEffect(() => {
    if (loaded && !showingCard) void setMainWindowTitle(SIGNED_IN_WINDOW_TITLE)
  }, [loaded, showingCard])

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
              title={LOAD_FAILED_TITLE}
              actions={<Btn onClick={() => void reader.retry()}>Try again</Btn>}
            />
          ) : (
            <div className="ms-loading" role="status">Loading…</div>
          )}
        </div>
      </div>
    )
  }

  const line = accountLine(account, plan)

  if (view.mode === 'main' && screen !== 'home') {
    return (
      <div className="mw-window mw-window--solo">
        <div className="mw-toolbar mw-toolbar--solo" data-tauri-drag-region="" />
        <main className="mw-solo-main" aria-labelledby="mw-title">
          <SignInWindow
            screen={screen}
            email={account.email}
            flow={flowNow}
            note={signOutNote}
            loadFailed={snapshotFailed}
            onRetryLoad={() => reader.refresh()}
            onBack={canLeaveSignInAgain(account, flowNow) ? () => setSignInFlow(IDLE_FLOW) : undefined}
            onFlow={setSignInFlow}
          />
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
          aria-label={`Back to ${backLabel(view.returnTo, screen === 'home')}`}
          onClick={() => go({ to: 'back' })}
        >
          <SettingsIcon name="back" size={14} strokeWidth={1.8} />
          {backLabel(view.returnTo, screen === 'home')}
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
      pane = (
        <HomePane
          snapshot={snapshot}
          onRefresh={() => reader.refresh()}
          feed={reviewFeedNow}
          onFeedChanged={refreshReviewFeed}
          onOpenReview={() => go({ to: 'review' })}
        />
      )
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
        {view.section === 'general' ? <GeneralTab settings={settings} signedIn={signedIn} /> : null}
        {view.section === 'account' ? <AccountTab flow={flow} flowView={flowView} flowOutcomes={flowOutcomes} onSetUpThisMac={() => {
          // Review P2: the card is drawn in `main`, so the click also leaves Settings — otherwise the Account tab stays up and
          // the recovery-phrase form the person just asked for never appears.
          go({ to: 'home' })
          moveFlow({ ...IDLE_FLOW, step: 'unlock' }, setSignInFlow)
        }} /> : null}
        {view.section === 'sync' ? <SyncTab settings={settings} /> : null}
        {view.section === 'security' ? <SecurityTab unlocked={unlocked} /> : null}
        {view.section === 'about' ? <AboutTab settings={settings} signedIn={signedIn} /> : null}
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
