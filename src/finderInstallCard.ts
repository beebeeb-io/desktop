/**
 * Task 1524 Issue 4 — pure decision logic for the "Install the Finder
 * location" onboarding card, extracted out of `Onboarding.tsx` the same way
 * `onboardingSignIn.ts` extracted the 2FA sign-in decision (task 1521): a
 * plain function that a component calls, testable with `bun:test` and no
 * Tauri runtime.
 *
 * Background: `install_finder_location` (src-tauri/src/lib.rs) now returns
 * `Ok` even when the Beebeeb File Provider domain exists but the user (or
 * macOS) has disabled it in System Settings -> General -> Login Items &
 * Extensions -> File Providers (`reason_category === "user_disabled"`) —
 * deliberately NOT an `Err`, so the frontend can distinguish this from a
 * real installation success by inspecting `value.installed`, not `result.ok`
 * alone. Treating `result.ok` as "done" (the pre-fix shape) would silently
 * advance onboarding past a step that never actually installed anything —
 * exactly the bug this module's tests guard against.
 */
import type { CommandResult, FinderInstallState } from './desktopApi'

export type FinderInstallCardState =
  | { kind: 'installed'; path: string | null }
  | { kind: 'user_disabled'; message: string }
  | { kind: 'error'; message: string }

const DEFAULT_USER_DISABLED_MESSAGE = 'Beebeeb is turned off in System Settings.'
const DEFAULT_ERROR_MESSAGE = 'Could not install the Finder location.'

export function classifyFinderInstallResult(result: CommandResult<FinderInstallState>): FinderInstallCardState {
  if (!result.ok) {
    return { kind: 'error', message: result.reason }
  }
  if (result.value.installed) {
    return { kind: 'installed', path: result.value.path ?? null }
  }
  if (result.value.reason_category === 'user_disabled') {
    return { kind: 'user_disabled', message: result.value.last_error ?? DEFAULT_USER_DISABLED_MESSAGE }
  }
  return { kind: 'error', message: result.value.last_error ?? DEFAULT_ERROR_MESSAGE }
}

/**
 * Whether a poll of `finder_domain_user_enabled` should trigger an automatic
 * re-attempt of `install_finder_location`. Only a transition INTO `true`
 * matters:
 * - `value === true` (enabled): retry — this is the whole point of polling
 *   while the "turned off in System Settings" card is shown.
 * - `value === false` (still disabled) or `value === null` (domain not
 *   registered — e.g. removed since the last check): keep waiting, do NOT
 *   retry — nothing has changed for the better.
 * - `ok === false` (lookup failed): never treat a failed lookup as "the user
 *   fixed it" — that would spam `addDomain` on every failed poll tick.
 */
export function shouldRetryAfterUserEnabledPoll(poll: CommandResult<boolean | null>): boolean {
  return poll.ok && poll.value === true
}

/**
 * Task 1670 — which of the "Install in Finder" / "Open in Finder" buttons the
 * Settings > Finder location pane (`src/pages/SyncFolder.tsx`) should show,
 * given the current `installed` flag from `finder_location_state`. Extracted
 * so the truthfulness rule is unit-testable without mounting the page.
 *
 * Symptom (Guus, 2026-09-30, real Mac): the pane showed the green "Installed"
 * pill, the amber "Install in Finder" button, AND "Open in Finder" all at the
 * same time. Root cause was in the VIEW, not the state: `finder_location_state`
 * (src-tauri/src/lib.rs) already returns the correct `installed` boolean —
 * `SyncFolder.tsx`'s button row simply rendered both buttons unconditionally,
 * never consulting it. This function is the single place that decision now
 * lives: installed -> only "Open in Finder", shown as the primary action;
 * not installed -> only "Install in Finder".
 */
export function finderLocationButtonPlan(installed: boolean): { showInstall: boolean; showOpen: boolean } {
  return installed ? { showInstall: false, showOpen: true } : { showInstall: true, showOpen: false }
}

/**
 * Task 1683 slice 5 / decision D1 — a failed Finder install is ONE inline error, never also a
 * toast.
 *
 * The defect (screenshot 1): `install_finder_location` saved its failure
 * (`finder_install_last_error`, read back by `finder_location_state` and painted as a red banner)
 * AND returned it as an `Err`, which the pane turned into a toast: the same string twice, and the
 * banner outlived the toast. A Finder install failure GATES "Open in Finder", so under the house
 * rule ("a transient action failure is a toast, a failure that gates a control is inline",
 * eslint-rules/no-ad-hoc-error-surface.mjs) it belongs inline. The backend now saves it and
 * returns it as a state; this helper ALSO folds a rejected command (a failure that was never
 * saved, e.g. validation before anything is persisted, or an older backend) into that same
 * state, so every caller renders the failure from one place and exactly once.
 *
 * `previous` supplies the fields a rejection cannot know (the Finder path), so the pane does not
 * lose them on a failed attempt.
 */
export function finderInstallStateAfterAttempt(
  result: CommandResult<FinderInstallState>,
  previous: FinderInstallState | null,
  unsupportedLabel: string,
): FinderInstallState {
  if (result.ok) return result.value
  return {
    ...(previous ?? { installed: false }),
    installed: false,
    status: 'error',
    last_error: result.unsupported ? unsupportedLabel : result.reason,
    reason_category: null,
  }
}

/**
 * The state to show while a NEW install attempt runs: the previous failure is cleared (it is no
 * longer true of the attempt in flight), everything else is kept. Without this the stale banner
 * from the last attempt stayed on screen next to the new one.
 */
export function finderInstallStateWhileAttempting(previous: FinderInstallState | null): FinderInstallState | null {
  if (!previous) return previous
  return {
    ...previous,
    status: previous.status === 'error' ? 'missing' : previous.status,
    last_error: null,
    reason_category: null,
  }
}
