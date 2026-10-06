/**
 * Every Finder-setup string on macOS (spec 2026-10-06 §6.2: "All copy lives in one frontend
 * module … keyed by reason"). Surfaces never write their own. One sentence and one action per
 * reason; "Try again" only inside a failure. Pinned by tests/finderSetupCopy.test.ts, which also
 * holds the design artefact to the same strings.
 */
import type { FinderFailureReason, FinderSetupLoad, FinderSetupState, FinderSetupView } from './finderSetup'

export type FinderSetupAction = 'try_again' | 'open_system_settings' | 'show_in_finder' | 'copy_details'

export const FINDER_SETUP_TITLE = 'Beebeeb in Finder'
export const FINDER_ADDING_LINE = 'Adding Beebeeb to Finder…'
export const FINDER_READY_LINE = 'Your vault appears under Locations in Finder.'

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

/** Toast titles for an action that failed (a failed action gates nothing, so it is a toast). */
export const FINDER_ACTION_FAILED: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'Couldn’t try again',
  open_system_settings: 'Couldn’t open System Settings',
  show_in_finder: 'Couldn’t show Beebeeb in Finder',
  copy_details: 'Couldn’t copy the details',
}

/** The short pill on the compact Status page (spec B deletes that page). */
export const FINDER_STATUS_PILL: Readonly<Record<FinderSetupState, string>> = {
  ready: 'Installed',
  adding: 'Adding',
  failed: 'Setup blocked',
  user_disabled: 'Turned off',
  missing: 'Checking',
}

/** The pill when there is no view yet, and when the state cannot be read (never "Adding", ruling 7a). */
export const FINDER_STATUS_PILL_LOADING = 'Loading'
export const FINDER_STATUS_PILL_UNAVAILABLE = 'Unknown'

export type FinderSetupPresentation =
  | { kind: 'quiet'; line: '' }
  | { kind: 'adding'; line: string }
  | { kind: 'ready'; line: string }
  | {
      kind: 'notice'
      tone: 'alert' | 'status'
      reason: FinderFailureReason
      sentence: string
      action: FinderSetupAction
      actionLabel: string
    }
  /**
   * The state could not be read. The one action is a re-read (`useFinderSetup().retry`), NOT the
   * `try_again` action, which asks the reconciler to run its check again.
   */
  | { kind: 'unavailable'; line: string; actionLabel: string }

/** One state, one presentation. `quiet` = the instant before the first check (or no view yet). */
export function finderSetupPresentation(view: FinderSetupView | null): FinderSetupPresentation {
  if (!view) return { kind: 'quiet', line: '' }
  if (view.setup === 'ready') return { kind: 'ready', line: FINDER_READY_LINE }
  if (view.setup === 'adding') return { kind: 'adding', line: FINDER_ADDING_LINE }
  const reason: FinderFailureReason | null =
    view.setup === 'user_disabled' ? 'user_disabled' : view.setup === 'failed' ? (view.reason ?? 'unknown') : view.reason
  if (!reason) return { kind: 'quiet', line: '' }
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
export function finderSetupLoadPresentation(load: FinderSetupLoad): FinderSetupPresentation {
  if (load.status === 'unavailable') {
    return { kind: 'unavailable', line: FINDER_UNAVAILABLE_LINE, actionLabel: FINDER_ACTION_LABEL.try_again }
  }
  return finderSetupPresentation(load.status === 'loaded' ? load.view : null)
}

/**
 * The compact pages' pill (SyncFolder, Status) for whatever the hook has: its label and the tone
 * the page paints its dot with. One place, so the two pages cannot drift.
 */
export function finderStatusPill(load: FinderSetupLoad): { label: string; tone: 'ok' | 'warn' | 'error' | 'idle' } {
  if (load.status === 'unavailable') return { label: FINDER_STATUS_PILL_UNAVAILABLE, tone: 'warn' }
  if (load.status === 'loading') return { label: FINDER_STATUS_PILL_LOADING, tone: 'idle' }
  const presentation = finderSetupPresentation(load.view)
  const tone = presentation.kind === 'ready' ? 'ok' : presentation.kind === 'notice' && presentation.tone === 'alert' ? 'error' : 'warn'
  return { label: FINDER_STATUS_PILL[load.view.setup], tone }
}
