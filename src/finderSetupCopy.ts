/**
 * Every Finder-setup string on macOS (spec 2026-10-06 §6.2: "All copy lives in one frontend
 * module … keyed by reason"). Surfaces never write their own. One sentence and one action per
 * reason; "Try again" only inside a failure. Pinned by tests/finderSetupCopy.test.ts, which also
 * holds the design artefact to the same strings.
 */
import type { EngineRefusal } from './desktopApi'
import type { FinderFailureReason, FinderSetupLoad, FinderSetupView } from './finderSetup'

export type FinderSetupAction = 'try_again' | 'open_system_settings' | 'show_in_finder' | 'copy_details'

export const FINDER_SETUP_TITLE = 'Beebeeb in Finder'
export const FINDER_ADDING_LINE = 'Adding Beebeeb to Finder…'
export const FINDER_READY_LINE = 'Your vault appears under Locations in Finder.'

/**
 * A loaded `Missing` (spec §5.2 as amended; lead ruling FA-I2, which corrects FT-I2's copy): no check
 * owns Finder right now, and Beebeeb may or may not still be registered. It rests there after a
 * sign-out, after a Lock that interrupts a check, and while the keys are not on this Mac. So it is a
 * quiet row: no pill, no activity, no claim about presence, and where a slot must show text, this one
 * sentence. It is also what "Try again" answers while the reconciler is held (`FINDER_SETUP_HELD` in
 * src-tauri/src/lib.rs, the same text). The wording is for Guus to confirm on #114.
 */
export const FINDER_RESTING_LINE = 'Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.'

/**
 * The onboarding step rail's entry for the Finder step, on macOS only (lead ruling on Task 15):
 * the rail must not promise a manual install, because on macOS nothing is installed by hand
 * (R5). Windows and Linux keep the rail's own words ("Install Finder location").
 */
export const FINDER_RAIL_TITLE = 'Finder'
export const FINDER_RAIL_DETAIL = 'Beebeeb adds itself to Finder after sign-in.'

/**
 * The state could not be read at all (lead ruling 7a). These are the words the Sync tab's
 * unavailable row already uses (`finderHint`, macSettingsModel.ts), kept here so that a surface
 * never composes its own; tests/finderSetupCopy.test.ts holds the two equal.
 */
export const FINDER_UNAVAILABLE_LINE = 'Couldn’t check Finder.'

/**
 * A failed "Open in Finder" on a Mac (task 17b, lead ruling T4-⚠2). Every macOS error from the File
 * Provider bridge that reaches the frontend is redacted to a domain and a code (ruling T1-⚠4: the OS
 * message can carry a path), so the code is never shown; a person reads this one sentence instead.
 */
export const FINDER_OPEN_FAILED = 'Beebeeb couldn’t open its Finder location.'

/**
 * A failed repair of the Finder location on a Mac (task 17b, lead ruling): the SyncFolder pane's Reset.
 * Same reason as `FINDER_OPEN_FAILED`: the bridge's error is a bare domain and code, never shown.
 */
export const FINDER_REPAIR_FAILED = 'Beebeeb couldn’t repair its Finder location.'

/**
 * A repair (Reset) that succeeded but could not clean up everything (task 17b, fix round 1). Rust
 * puts a bridge error code and a cache-file path in the result's `warnings`; neither is ever shown on
 * a Mac, so any warning turns the notice into this one sentence.
 */
export const FINDER_REPAIR_PARTIAL = 'Beebeeb repaired its Finder location, but couldn’t remove everything it left behind.'

/**
 * A failed "show this file in Finder" (`open_in_finder`: the Shared roots page, quick search) on a
 * Mac. Same reason as `FINDER_OPEN_FAILED`: the error is never shown, this sentence is.
 */
export const FINDER_SHOW_FILE_FAILED = 'Beebeeb couldn’t show that file in Finder.'

/**
 * The one sentence a repair result's warnings turn into on a Mac, or `null` when there is nothing to
 * say. It takes the whole result so that no caller reads `.warnings` itself, and it never returns any
 * of the warning text.
 */
export function finderRepairWarningNote(result: { warnings: readonly string[] }): string | null {
  return result.warnings.some((warning) => warning.trim().length > 0) ? FINDER_REPAIR_PARTIAL : null
}

export const FINDER_REASON_COPY: Readonly<Record<FinderFailureReason, { sentence: string; action: FinderSetupAction }>> = {
  extension_loading: { sentence: 'macOS hasn’t finished loading Beebeeb’s Finder extension.', action: 'try_again' },
  user_disabled: { sentence: 'Beebeeb is turned off in System Settings.', action: 'open_system_settings' },
  not_in_applications: { sentence: 'Beebeeb is running from the disk image. Move it to Applications, then open it again.', action: 'show_in_finder' },
  folder_taken: {
    sentence: 'An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.',
    action: 'copy_details',
  },
  signing: { sentence: 'This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.', action: 'copy_details' },
  timeout: { sentence: 'macOS didn’t finish adding Beebeeb to Finder.', action: 'try_again' },
  unknown: { sentence: 'Beebeeb couldn’t be added to Finder.', action: 'try_again' },
}

export const FINDER_ACTION_LABEL: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'Try again',
  open_system_settings: 'Open System Settings',
  show_in_finder: 'Show in Finder',
  copy_details: 'Copy details',
}

/**
 * The one sentence a failed action says (a failed action gates nothing, so it is a toast, and the
 * toast is this sentence alone: no title, never the reason). Task 17b, lead ruling: every macOS error
 * from the File Provider bridge that reaches the frontend is a bare domain and code, and a person
 * must not read it. Not for the reconciler's own failures: those are `FINDER_REASON_COPY`.
 */
export const FINDER_ACTION_FAILED: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'Beebeeb couldn’t retry adding itself to Finder.',
  open_system_settings: 'Beebeeb couldn’t open System Settings.',
  show_in_finder: 'Beebeeb couldn’t show itself in Finder.',
  copy_details: 'Beebeeb couldn’t copy the details.',
}

/**
 * The short pill on the compact Status and SyncFolder pages (spec B deletes them), keyed on what the
 * surface PRESENTS, not on the raw state (lead ruling FT-I2): a notice says its own label whatever
 * state it came from (a `missing` that carries `not_in_applications` is "Setup blocked", never
 * "Checking"), and a loaded `Missing` has no pill at all (FA-I2).
 */
export type FinderStatusPillKey = 'ready' | 'adding' | 'blocked' | 'turned_off'
export const FINDER_STATUS_PILL: Readonly<Record<FinderStatusPillKey, string>> = {
  ready: 'Installed',
  adding: 'Adding',
  blocked: 'Setup blocked',
  turned_off: 'Turned off',
}

/** The pill when there is no view yet, and when the state cannot be read (never "Adding", ruling 7a). */
export const FINDER_STATUS_PILL_LOADING = 'Loading'
export const FINDER_STATUS_PILL_UNAVAILABLE = 'Unknown'

export type FinderSetupPresentation =
  /** No view yet: nothing true to say. */
  | { kind: 'quiet'; line: '' }
  /** A loaded `Missing` (FA-I2): one sentence that claims no activity and nothing about presence. */
  | { kind: 'resting'; line: string }
  | { kind: 'adding'; line: string }
  | { kind: 'ready'; line: string }
  | {
      kind: 'notice'
      tone: 'alert' | 'status'
      reason: FinderFailureReason
      sentence: string
      /** `null` only for an `engine_stop_unconfirmed` refusal: nothing but a relaunch helps. */
      action: FinderSetupAction | null
      actionLabel: string | null
    }
  /**
   * The state could not be read. The one action is a re-read (`useFinderSetup().retry`), NOT the
   * `try_again` action, which asks the reconciler to run its check again.
   */
  | { kind: 'unavailable'; line: string; actionLabel: string }

/**
 * One state, one presentation. `quiet` = no view yet. A loaded `Missing` without a reason is
 * `resting` (FA-I2); with a reason (D7: running from the disk image) it is that reason's notice.
 *
 * `refusal` is `sync_status.engine_refusal` (must-render row 9, FT-I5). When the engine start was
 * refused, the reconciler's re-add fails as `unknown`, whose sentence names the wrong cause; so a
 * `failed` + `unknown` with a refusal says the refusal's sentence. Its action stays Try again (the
 * retry re-identifies the session), except for `engine_stop_unconfirmed`, which has no action because
 * only quitting and reopening Beebeeb helps. Every other state ignores the refusal.
 */
export function finderSetupPresentation(view: FinderSetupView | null, refusal: EngineRefusal | null = null): FinderSetupPresentation {
  if (!view) return { kind: 'quiet', line: '' }
  if (view.setup === 'ready') return { kind: 'ready', line: FINDER_READY_LINE }
  if (view.setup === 'adding') return { kind: 'adding', line: FINDER_ADDING_LINE }
  const reason: FinderFailureReason | null =
    view.setup === 'user_disabled' ? 'user_disabled' : view.setup === 'failed' ? (view.reason ?? 'unknown') : view.reason
  if (!reason) return { kind: 'resting', line: FINDER_RESTING_LINE }
  if (view.setup === 'failed' && reason === 'unknown' && refusal) {
    const relaunchOnly = refusal.code === 'engine_stop_unconfirmed'
    return {
      kind: 'notice',
      tone: 'alert',
      reason,
      sentence: refusal.sentence,
      action: relaunchOnly ? null : 'try_again',
      actionLabel: relaunchOnly ? null : FINDER_ACTION_LABEL.try_again,
    }
  }
  const copy = FINDER_REASON_COPY[reason]
  return {
    kind: 'notice',
    tone: reason === 'user_disabled' ? 'status' : 'alert',
    reason,
    sentence: copy.sentence,
    action: copy.action,
    actionLabel: FINDER_ACTION_LABEL[copy.action],
  }
}

/** The presentation of a load that may not have succeeded (lead ruling 7a: never "Adding"). */
export function finderSetupLoadPresentation(load: FinderSetupLoad, refusal: EngineRefusal | null = null): FinderSetupPresentation {
  if (load.status === 'unavailable') {
    return { kind: 'unavailable', line: FINDER_UNAVAILABLE_LINE, actionLabel: FINDER_ACTION_LABEL.try_again }
  }
  return finderSetupPresentation(load.status === 'loaded' ? load.view : null, refusal)
}

/**
 * The compact pages' pill (SyncFolder, Status) for whatever the hook has: its label and the tone
 * the page paints its dot with, or `null` for a loaded `Missing`, which has no pill (FA-I2). Keyed on
 * the presentation (FT-I2). One place, so the two pages cannot drift.
 */
export function finderStatusPill(load: FinderSetupLoad): { label: string; tone: 'ok' | 'warn' | 'error' | 'idle' } | null {
  if (load.status === 'unavailable') return { label: FINDER_STATUS_PILL_UNAVAILABLE, tone: 'warn' }
  if (load.status === 'loading') return { label: FINDER_STATUS_PILL_LOADING, tone: 'idle' }
  const presentation = finderSetupPresentation(load.view)
  switch (presentation.kind) {
    case 'ready':
      return { label: FINDER_STATUS_PILL.ready, tone: 'ok' }
    case 'adding':
      return { label: FINDER_STATUS_PILL.adding, tone: 'warn' }
    case 'notice':
      return presentation.tone === 'alert'
        ? { label: FINDER_STATUS_PILL.blocked, tone: 'error' }
        : { label: FINDER_STATUS_PILL.turned_off, tone: 'warn' }
    default:
      return null
  }
}
