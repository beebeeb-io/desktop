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
import { command, commandUnavailableLabel, type CommandResult } from './desktopApi'
import {
  FINDER_ACTION_FAILED,
  finderSetupLoadPresentation,
  type FinderSetupAction,
  type FinderSetupPresentation,
} from './finderSetupCopy'
import { useToast } from './windows/ui'

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

/** The command each action sends, for an honest "not wired" label. */
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

export interface FinderActionDeps {
  writeClipboard?: (text: string) => Promise<void>
}

export async function copyFinderSetupDetails(deps: FinderActionDeps = {}): Promise<CommandResult<void>> {
  const details = await command<string>('finder_setup_copy_details')
  if (!details.ok) return details
  const write = deps.writeClipboard ?? ((text: string) => navigator.clipboard.writeText(text))
  try {
    await write(details.value)
    return { ok: true, value: undefined }
  } catch {
    return { ok: false, reason: 'The details could not be put on the pasteboard.', unsupported: false }
  }
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
  /** What to render, from `finderSetupCopy`. Never "Adding" unless the reconciler said so. */
  presentation: FinderSetupPresentation
  /** Read the state again. It is the one action of the `unavailable` presentation. */
  retry: () => Promise<void>
  /**
   * Run one action. A failed action raises an error toast (it gates nothing) and is also
   * returned, so a surface may react to success, e.g. to confirm that details were copied.
   */
  run: (action: FinderSetupAction) => Promise<CommandResult<void>>
}

/**
 * The one place a surface gets the Finder setup from (lead ruling 7b).
 *
 * - It subscribes to `finder-setup-changed`, and reads `finder_setup_state` once the subscription
 *   has landed, so no transition falls between the two. An event that arrives while a read is in
 *   flight wins over that read, because the read may be the older of the two.
 * - A state that cannot be read (a rejected command, an unparsable shape) is `unavailable`, never
 *   "Adding" (ruling 7a); `retry` reads again.
 * - A failed action is one error toast, titled for the action.
 * - It unsubscribes on unmount and ignores anything that finishes afterwards.
 */
export function useFinderSetup(deps: FinderActionDeps = {}): FinderSetupController {
  const { showToast } = useToast()
  const writeClipboard = deps.writeClipboard
  const [load, setLoad] = useState<FinderSetupLoad>({ status: 'loading' })
  const eventsSeen = useRef(0)
  const alive = useRef(false)

  const read = useCallback(async () => {
    const eventsBefore = eventsSeen.current
    const result = await loadFinderSetup()
    if (!alive.current || eventsSeen.current !== eventsBefore) return
    setLoad(result.ok ? { status: 'loaded', view: result.value } : { status: 'unavailable' })
  }, [])

  const retry = useCallback(async () => {
    setLoad({ status: 'loading' })
    await read()
  }, [read])

  useEffect(() => {
    alive.current = true
    const stop = subscribeFinderSetup(
      (view) => {
        eventsSeen.current += 1
        setLoad({ status: 'loaded', view })
      },
      {
        onInvalid: () => void read(),
        onSubscribed: () => void read(),
      },
    )
    return () => {
      alive.current = false
      stop()
    }
  }, [read])

  const run = useCallback(
    async (action: FinderSetupAction): Promise<CommandResult<void>> => {
      const result = await runFinderSetupAction(action, { writeClipboard })
      if (!result.ok) {
        showToast({
          variant: 'error',
          title: FINDER_ACTION_FAILED[action],
          message: result.unsupported ? commandUnavailableLabel(FINDER_ACTION_COMMAND[action]) : result.reason,
        })
      }
      return result
    },
    [writeClipboard, showToast],
  )

  return { load, presentation: finderSetupLoadPresentation(load), retry, run }
}
