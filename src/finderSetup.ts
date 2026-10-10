/**
 * macOS Finder setup (spec docs/specs/2026-10-06-macos-finder-setup-reconciler.md). The reconciler
 * in Rust (src-tauri/src/finder_setup/) owns the state. Surfaces read it (`finder_setup_state`),
 * follow `finder-setup-changed`, and offer the one action a state allows. No surface on macOS
 * installs anything (ruling R5). The shapes mirror `FinderSetupView` in finder_setup/driver.rs.
 *
 * Surfaces do not call the commands below one by one: `useFinderSetup()` holds the load, the
 * subscription and the failed-action toast once (lead ruling 7b), and returns what to render.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, loadEngineRefusal, type CommandResult, type EngineRefusal } from './desktopApi'
import {
  FINDER_ACTION_FAILED,
  FINDER_COPIED_MS,
  FINDER_OPEN_FAILED,
  FINDER_REPAIR_FAILED,
  FINDER_SHOW_FILE_FAILED,
  finderSetupLoadPresentation,
  type FinderSetupAction,
  type FinderSetupPresentation,
} from './finderSetupCopy'
import { useToast, type ToastInput } from './windows/ui'

export const FINDER_FAILURE_REASONS = [
  'extension_loading',
  'user_disabled',
  'not_in_applications',
  'folder_taken',
  'signing',
  'timeout',
  'unknown',
] as const
export type FinderFailureReason = (typeof FINDER_FAILURE_REASONS)[number]
export type FinderSetupState = 'ready' | 'missing' | 'adding' | 'failed' | 'user_disabled'
export type LaunchLocation = 'applications' | 'translocated' | 'disk_image' | 'elsewhere'

export interface FinderFailureRecord {
  reason: FinderFailureReason
  domain: string
  code: number
  at: number
}

export interface FinderSetupView {
  setup: FinderSetupState
  reason: FinderFailureReason | null
  launch_location: LaunchLocation
  attempt: number
  max_attempts: number
  last_failure: FinderFailureRecord | null
}

export const FINDER_SETUP_CHANGED_EVENT = 'finder-setup-changed'

/** The command each action sends. Documentation of the wiring; no surface shows it to a person. */
export const FINDER_ACTION_COMMAND: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'finder_setup_retry',
  open_system_settings: 'open_login_items_and_extensions_settings',
  show_in_finder: 'finder_setup_show_app',
  copy_details: 'finder_setup_copy_details',
}

const SETUP_STATES: readonly FinderSetupState[] = ['ready', 'missing', 'adding', 'failed', 'user_disabled']
const LAUNCH_LOCATIONS: readonly LaunchLocation[] = ['applications', 'translocated', 'disk_image', 'elsewhere']

function isOneOf<T extends string>(allowed: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function parseFailureRecord(value: unknown): FinderFailureRecord | null {
  if (!isRecord(value)) return null
  const { reason, domain, code, at } = value
  if (!isOneOf(FINDER_FAILURE_REASONS, reason) || typeof domain !== 'string') return null
  if (typeof code !== 'number' || !Number.isFinite(code) || typeof at !== 'number' || !Number.isFinite(at)) return null
  return { reason, domain, code, at }
}

/**
 * The reconciler's `FinderSetupView`, or `null` when the value is not one. The vocabulary is
 * closed on both sides, so a value outside it is a contract break to surface (the unavailable
 * state), not something to guess a meaning for.
 */
export function parseFinderSetupView(value: unknown): FinderSetupView | null {
  if (!isRecord(value)) return null
  const { setup, reason, launch_location, attempt, max_attempts, last_failure } = value
  if (!isOneOf(SETUP_STATES, setup) || !isOneOf(LAUNCH_LOCATIONS, launch_location)) return null
  const isCount = (n: unknown): n is number => typeof n === 'number' && Number.isInteger(n) && n >= 0
  if (!isCount(attempt) || !isCount(max_attempts)) return null
  let parsedReason: FinderFailureReason | null = null
  if (reason != null) {
    if (!isOneOf(FINDER_FAILURE_REASONS, reason)) return null
    parsedReason = reason
  }
  let parsedFailure: FinderFailureRecord | null = null
  if (last_failure != null) {
    parsedFailure = parseFailureRecord(last_failure)
    if (!parsedFailure) return null
  }
  return { setup, reason: parsedReason, launch_location, attempt, max_attempts, last_failure: parsedFailure }
}

export async function loadFinderSetup(): Promise<CommandResult<FinderSetupView>> {
  const result = await command<unknown>('finder_setup_state')
  if (!result.ok) return result
  const view = parseFinderSetupView(result.value)
  return view
    ? { ok: true, value: view }
    : { ok: false, reason: 'Finder setup returned a state of an unexpected shape.', unsupported: false }
}

export interface FinderSubscribeOptions {
  /** A payload that was not a `FinderSetupView` arrived: read the state again instead of guessing. */
  onInvalid?: () => void
  /**
   * The listener is registered, or could not be (no event bus). Read the state after this, never
   * before: a transition between the read and the registration would otherwise be lost for good.
   */
  onSubscribed?: () => void
}

/** `unlisten` is typed `() => void` but is async underneath; a late failure to unlisten is noise. */
function release(unlisten: () => void): void {
  try {
    void Promise.resolve(unlisten()).catch(() => {})
  } catch {
    // nothing left to release
  }
}

/**
 * Rebase re-review I2: Rust emits this once it has saved a kept folder for the Settings › Sync row
 * (`surface_kept_folder` in `src-tauri/src/lib.rs`, its `KEPT_FOLDER_CHANGED_EVENT`; a test pins the two names equal).
 * A removal the reconciler runs in the background (after "Try again", an owed one, or one that finished after its
 * time limit) saves the folder long after the row read it. No payload: the row reads the folder through
 * `kept_unsynced_folder`.
 */
export const KEPT_FOLDER_CHANGED_EVENT = 'kept-folder-changed'

/**
 * Follow `kept-folder-changed`. `onSubscribed` as for {@link subscribeFinderSetup}: read the folder after it, never
 * before, or a folder saved between the read and the registration is missed. Returns the unsubscribe.
 */
export function subscribeKeptFolderChanged(
  onChanged: () => void,
  options: Pick<FinderSubscribeOptions, 'onSubscribed'> = {},
): () => void {
  let closed = false
  let stop: (() => void) | null = null
  listen<unknown>(KEPT_FOLDER_CHANGED_EVENT, () => {
    if (!closed) onChanged()
  }).then(
    (unlisten) => {
      if (closed) {
        release(unlisten)
        return
      }
      stop = unlisten
      options.onSubscribed?.()
    },
    () => {
      // No event bus (a webview without Tauri): the surface reads what it has, once.
      if (!closed) options.onSubscribed?.()
    },
  )
  return () => {
    closed = true
    if (stop) release(stop)
  }
}

/** Follow every reconciler transition. Returns the unsubscribe for an effect's cleanup. */
export function subscribeFinderSetup(onView: (view: FinderSetupView) => void, options: FinderSubscribeOptions = {}): () => void {
  let closed = false
  let stop: (() => void) | null = null
  listen<unknown>(FINDER_SETUP_CHANGED_EVENT, (event) => {
    if (closed) return
    const view = parseFinderSetupView(event.payload)
    if (view) onView(view)
    else options.onInvalid?.()
  }).then(
    (unlisten) => {
      if (closed) {
        release(unlisten)
        return
      }
      stop = unlisten
      options.onSubscribed?.()
    },
    () => {
      // No event bus (a webview without Tauri): the surface keeps the view it loads.
      if (!closed) options.onSubscribed?.()
    },
  )
  return () => {
    closed = true
    if (stop) release(stop)
    stop = null
  }
}

/**
 * The toast for a failed "Open in Finder" on a Mac (task 17b, lead ruling T4-⚠2). A failed action
 * that gates nothing is a toast, and on a Mac its text is the one sentence and nothing else:
 * `open_finder_location`'s own error is a bare code such as `io.beebeeb.bridge 3` (ruling T1-⚠4), so
 * the reason is deliberately not an input here and cannot be rendered by a surface. Windows and Linux
 * do not call this; their error text names a folder a person can act on and keeps its own title.
 */
export function finderOpenFailedToast(): ToastInput {
  return { variant: 'error', message: FINDER_OPEN_FAILED }
}

/** The toast for a failed repair (Reset) of the Finder location on a Mac: the one sentence, never the reason. */
export function finderRepairFailedToast(): ToastInput {
  return { variant: 'error', message: FINDER_REPAIR_FAILED }
}

/** The toast for a failed "show this file in Finder" on a Mac: the one sentence, never the reason. */
export function finderShowFileFailedToast(): ToastInput {
  return { variant: 'error', message: FINDER_SHOW_FILE_FAILED }
}

export interface FinderActionDeps {
  /**
   * The pasteboard write (a test seam; default `writeTextToPasteboard`). It is called synchronously in
   * the click, with the text as a promise, because the details are still on their way (FT-clipboard).
   */
  writeClipboard?: (text: Promise<string>) => Promise<void>
}

/** What `writeTextToPasteboard` writes with: the platform's clipboard and `ClipboardItem`, injectable for tests. */
export interface PasteboardEnv {
  clipboard: { write?: (items: never[]) => Promise<void>; writeText: (text: string) => Promise<void> }
  ClipboardItem?: new (items: Record<string, Promise<Blob>>) => unknown
}

function platformPasteboard(): PasteboardEnv {
  return {
    clipboard: navigator.clipboard as unknown as PasteboardEnv['clipboard'],
    ClipboardItem: typeof ClipboardItem === 'undefined' ? undefined : (ClipboardItem as unknown as PasteboardEnv['ClipboardItem']),
  }
}

/**
 * Put text on the pasteboard from a click (lead ruling FT-clipboard). WebKit allows a pasteboard write
 * only inside the click's transient activation, and an await on the IPC call that fetches the text
 * would lose it. So this must be called SYNCHRONOUSLY in the click handler: it starts
 * `navigator.clipboard.write([new ClipboardItem({'text/plain': <promise>})])` at once (WebKit accepts a
 * promise-valued item), and falls back to `writeText` when that is refused or `ClipboardItem` is
 * missing. Device check D0 confirms it on a Mac; if D0 shows NotAllowedError, a pbcopy command is next.
 */
export function writeTextToPasteboard(text: Promise<string>, env: PasteboardEnv = platformPasteboard()): Promise<void> {
  const { clipboard, ClipboardItem: Item } = env
  if (Item && typeof clipboard.write === 'function') {
    const blob = text.then((value) => new Blob([value], { type: 'text/plain' }))
    blob.catch(() => {})
    let started: Promise<void>
    try {
      started = clipboard.write([new Item({ 'text/plain': blob }) as never])
    } catch (error) {
      started = Promise.reject(error)
    }
    return started.catch(async () => clipboard.writeText(await text))
  }
  return text.then((value) => clipboard.writeText(value))
}

export interface FinderSetupOptions extends FinderActionDeps {
  /**
   * Default true. A surface that also exists on Windows and Linux (SyncFolder, Status) cannot call
   * a hook conditionally, so it calls this always and passes whether the host is a Mac. While it is
   * false the hook reads nothing, listens to nothing and presents nothing (`quiet`), so off macOS the
   * reconciler's command and event are never touched.
   */
  enabled?: boolean
}

/**
 * "Copy details": fetch the text and put it on the pasteboard. The write starts before the text has
 * come back (synchronously, still inside the click; see `writeTextToPasteboard`). A failed details
 * command is returned as is; a refused pasteboard is a plain failed result, for the surface to toast.
 */
export function copyFinderSetupDetails(deps: FinderActionDeps = {}): Promise<CommandResult<void>> {
  const details = command<string>('finder_setup_copy_details')
  const text = details.then((answer) => (answer.ok ? answer.value : Promise.reject(new Error('no details'))))
  text.catch(() => {})
  const write = deps.writeClipboard ?? ((pending: Promise<string>) => writeTextToPasteboard(pending))
  let written: Promise<void>
  try {
    written = write(text)
  } catch (error) {
    written = Promise.reject(error)
  }
  written.catch(() => {})
  return details.then(async (answer): Promise<CommandResult<void>> => {
    if (!answer.ok) return answer
    try {
      await written
      return { ok: true, value: undefined }
    } catch {
      return { ok: false, reason: 'The details could not be put on the pasteboard.', unsupported: false }
    }
  })
}

export function runFinderSetupAction(action: FinderSetupAction, deps: FinderActionDeps = {}): Promise<CommandResult<void>> {
  switch (action) {
    case 'try_again':
      return command<void>('finder_setup_retry')
    case 'open_system_settings':
      return command<void>('open_login_items_and_extensions_settings')
    case 'show_in_finder':
      return command<void>('finder_setup_show_app')
    case 'copy_details':
      return copyFinderSetupDetails(deps)
  }
}

/** What the hook has: nothing yet, a view, or no way to read one. */
export type FinderSetupLoad =
  | { status: 'loading' }
  | { status: 'loaded'; view: FinderSetupView }
  | { status: 'unavailable' }

export interface FinderSetupController {
  load: FinderSetupLoad
  /**
   * `sync_status.engine_refusal` as of the last read (on load and on every `finder-setup-changed`;
   * there is no event of its own), or null. Already folded into `presentation`.
   */
  refusal: EngineRefusal | null
  /** What to render, from `finderSetupCopy`. Never "Adding" unless the reconciler said so. */
  presentation: FinderSetupPresentation
  /** Read the state again. It is the one action of the `unavailable` presentation. */
  retry: () => Promise<void>
  /**
   * Run one action. A failed action raises one error toast with the action's one sentence (it
   * gates nothing; the reason is never shown) and is also returned, so a surface may react to
   * success, e.g. to confirm that details were copied. Try again is the exception: see `actionNote`.
   */
  run: (action: FinderSetupAction) => Promise<CommandResult<void>>
  /**
   * What a failed Try again said (must-render row 15), or null. `finder_setup_retry` fails only with
   * Rust's fixed sentences (the reconciler held: `FINDER_SETUP_HELD`; not running: `NOT_RUNNING`), and
   * each carries the remedy, so it is shown verbatim as a neutral status line, never under "Couldn’t
   * retry". It goes when the state changes or the next action runs.
   */
  actionNote: string | null
  /** A Copy details just put the details on the pasteboard: the button says "Copied" for a moment. */
  copied: boolean
}

/**
 * The one place a surface gets the Finder setup from (lead ruling 7b).
 *
 * - It subscribes to `finder-setup-changed`, and reads `finder_setup_state` once the subscription
 *   has landed, so no transition falls between the two. An event that arrives while a read is in
 *   flight wins over that read, because the read may be the older of the two, and of two reads in
 *   flight only the newest issued may land (M1, triage 17).
 * - It reads `sync_status.engine_refusal` at the same moment and again on every event (a refusal has
 *   no event of its own), and only the newest of those reads counts (must-render row 9).
 * - A state that cannot be read (a rejected command, an unparsable shape) is `unavailable`, never
 *   "Adding" (ruling 7a); `retry` reads again. A re-read blanks the row only from `unavailable`; a
 *   view that is there stays on screen while it is read again (M1, triage 19).
 * - A failed action is one error toast: the action's one sentence, no title, never the reason. It
 *   leaves a console trace of the action and a reason CODE only (17b-M3).
 * - It unsubscribes on unmount and ignores anything that finishes afterwards.
 * - `enabled: false` (a surface on a host that is not a Mac) does none of the above.
 */
export function useFinderSetup(options: FinderSetupOptions = {}): FinderSetupController {
  const { showToast } = useToast()
  const { enabled = true, writeClipboard } = options
  const [stored, setLoad] = useState<FinderSetupLoad>({ status: 'loading' })
  const [refusal, setRefusal] = useState<EngineRefusal | null>(null)
  const [actionNote, setActionNote] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const eventsSeen = useRef(0)
  const readsIssued = useRef(0)
  const refusalReads = useRef(0)
  const alive = useRef(false)

  // The newest read wins: an older answer that lands after a newer one was issued is dropped.
  const readRefusal = useCallback(async () => {
    const issued = ++refusalReads.current
    const next = await loadEngineRefusal()
    if (!alive.current || issued !== refusalReads.current) return
    setRefusal(next)
  }, [])

  const read = useCallback(async () => {
    const eventsBefore = eventsSeen.current
    const issued = ++readsIssued.current
    const result = await loadFinderSetup()
    if (!alive.current || eventsSeen.current !== eventsBefore || issued !== readsIssued.current) return
    setLoad(result.ok ? { status: 'loaded', view: result.value } : { status: 'unavailable' })
  }, [])

  const retry = useCallback(async () => {
    if (!enabled) return
    // Blank only what could not be read; a view that is there stays while it is read again.
    setLoad((current) => (current.status === 'unavailable' ? { status: 'loading' } : current))
    void readRefusal()
    await read()
  }, [read, readRefusal, enabled])

  useEffect(() => {
    if (!enabled) return
    alive.current = true
    const stop = subscribeFinderSetup(
      (view) => {
        eventsSeen.current += 1
        setLoad({ status: 'loaded', view })
        setActionNote(null)
        void readRefusal()
      },
      {
        onInvalid: () => {
          void read()
          void readRefusal()
        },
        onSubscribed: () => {
          void read()
          void readRefusal()
        },
      },
    )
    return () => {
      alive.current = false
      stop()
      if (copiedTimer.current !== null) clearTimeout(copiedTimer.current)
    }
  }, [read, readRefusal, enabled])

  const run = useCallback(
    async (action: FinderSetupAction): Promise<CommandResult<void>> => {
      setActionNote(null)
      setCopied(false)
      if (copiedTimer.current !== null) clearTimeout(copiedTimer.current)
      // Synchronous up to the pasteboard write: Copy details must start it inside the click.
      const result = await runFinderSetupAction(action, { writeClipboard })
      if (result.ok && action === 'copy_details' && alive.current) {
        setCopied(true)
        copiedTimer.current = setTimeout(() => {
          if (alive.current) setCopied(false)
        }, FINDER_COPIED_MS)
      }
      if (!result.ok) {
        if (action === 'try_again' && !result.unsupported) {
          // Row 15 (census exemption 3, pinned by tests/finderSetupSourceContract.test.ts): Rust's
          // `finder_setup_retry` fails only with fixed sentences that carry the remedy, so it is said
          // verbatim and neutrally, never hidden behind a fixed "couldn't retry".
          setActionNote(result.reason)
        } else {
          // One fixed sentence per action, never `result.reason` (task 17b): on a Mac that is a
          // redacted bridge code. An `unsupported` failure says the same sentence, because that flag
          // is a substring guess on the reason and must not put a command name in front of a person.
          showToast({ variant: 'error', message: FINDER_ACTION_FAILED[action] })
          // 17b-M3: the dropped reason leaves a trace for diagnostics, as a CODE only: a redacted
          // `domain code` pair as it is, `unsupported`, or `other` for any free text (it could hold a path).
          const said = result.reason.trim()
          console.warn(action, result.unsupported ? 'unsupported' : /^[A-Za-z][\w.]*\s-?\d+$/.test(said) ? said : 'other')
        }
      }
      return result
    },
    [writeClipboard, showToast],
  )

  // A disabled hook shows nothing, even if it was enabled a moment ago and holds a stale view.
  const load: FinderSetupLoad = enabled ? stored : { status: 'loading' }
  const shownRefusal = enabled ? refusal : null
  return {
    load,
    refusal: shownRefusal,
    presentation: finderSetupLoadPresentation(load, shownRefusal),
    retry,
    run,
    actionNote: enabled ? actionNote : null,
    copied: enabled && copied,
  }
}
