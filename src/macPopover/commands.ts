/**
 * Which backend commands the popover's buttons call (task 1683 slice 3).
 *
 * Seven of the actions use commands that exist today. THREE do not exist yet; they are named
 * here so the contract is written down once and slice 6 (the flip) implements exactly these.
 * Until then each is called through `command()`, which turns a missing command into
 * `{ ok: false, unsupported: true }`, so a build that flips the popover on before they exist
 * fails loudly on the button instead of doing nothing.
 *
 * `PENDING_BACKEND_COMMANDS` is checked by `tests/macPopoverCommands.test.ts` against the
 * `generate_handler!` list in `src-tauri/src/lib.rs`: a name in this list that IS registered, or
 * a name used by the popover that is neither registered nor listed here, fails the test. The
 * list is therefore the honest record of what slice 6 still owes; delete a name when it lands.
 *
 * Decision record: `.claude/tasks/decisions/1683-s3-popover-backend-gaps.md`.
 */

/** Commands that exist in `generate_handler!` today. */
export const WIRED_COMMANDS = {
  resumeSync: 'tray_resume_sync',
  installFinder: 'install_finder_location',
  openFinder: 'open_finder_location',
  openSystemSettings: 'open_login_items_and_extensions_settings',
  openOnboarding: 'open_onboarding_window',
  openConflict: 'open_conflict_window',
  hideWindow: 'plugin:window|hide',
} as const

/** Commands the popover needs that nothing implements yet (slice 6). */
export const PENDING_COMMANDS = {
  /**
   * Pops the native NSMenu (spec section 4 "Gear menu") under the gear. Args: `{ x, y }`, the
   * bottom-right corner of the gear button in CSS pixels (= logical points at zoom 1) from the
   * popover's top-left. Rust owns the items so the right-click menu on the icon is the same menu.
   */
  gearMenu: 'popover_gear_menu',
  /**
   * Unlock the vault with the account password (state c1b). Args: `{ password }`. Resolves on
   * success; rejects with the string `wrong_password` when the password is refused and with
   * anything else for any other failure. The existing `unlock_vault` takes NO password: it
   * restores the Keychain session, so the password field has nothing to verify against it.
   */
  unlockWithPassword: 'popover_unlock_vault',
  /** Run a sync tick now (state d `Try again`). No args. Without it `Try again` only re-reads the status. */
  retrySync: 'popover_retry_sync',
} as const

export const PENDING_BACKEND_COMMANDS: readonly string[] = Object.values(PENDING_COMMANDS)

export const VIEW_ONLINE_URL = 'https://app.beebeeb.io'
