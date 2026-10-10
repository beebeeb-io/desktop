/**
 * Pure logic behind the macOS Settings window (task 1683 slice 4; spec
 * `docs/specs/2026-09-30-macos-menubar-popover.md` section 5, artboards j/k/l/m).
 *
 * Nothing here touches React or Tauri, so `tests/macSettingsModel.test.ts` can pin every
 * decision the window makes about what to say: which row shows which state, which sentence
 * a Finder failure gets, how a speed limit is written, what the plan line reads.
 *
 * The window itself (`MacSettings.tsx`) is mounted only at `?window=settings-v2&platform=macos`
 * until slice 6 flips macOS to it; nothing rendered today imports this file.
 */
import type { FinderInstallState, Subscription, VaultItem } from './desktopApi'
import { finderInstallNotice } from './finderInstallCard'
import { quotaPercent, titleCasePlan, planRenewalCopy } from './planPresentation'
import type { ManualUpdateCheckState } from './windows/updateCheckViewModel'
import { formatStorageUsage } from './storageFormat'

// ── Tabs ────────────────────────────────────────────────────────────────────

export const SETTINGS_TABS = [
  { id: 'general', label: 'General', icon: 'settings' },
  { id: 'account', label: 'Account', icon: 'users' },
  { id: 'sync', label: 'Sync', icon: 'refresh' },
  { id: 'about', label: 'About', icon: 'info' },
] as const

export type SettingsTab = (typeof SETTINGS_TABS)[number]['id']

export const DEFAULT_SETTINGS_TAB: SettingsTab = 'general'

export function parseSettingsTab(value: string | null | undefined): SettingsTab | null {
  return SETTINGS_TABS.find((tab) => tab.id === value)?.id ?? null
}

/** `?tab=sync` deep-links a tab; anything else opens the first one. */
export function settingsTabFromSearch(search: string): SettingsTab {
  return parseSettingsTab(new URLSearchParams(search).get('tab')) ?? DEFAULT_SETTINGS_TAB
}

/** The tab the window was opened on: `?tab=` of the page it is showing, else General. */
export function settingsTabFromLocation(): SettingsTab {
  try {
    return settingsTabFromSearch(globalThis.location?.search ?? '')
  } catch {
    return DEFAULT_SETTINGS_TAB
  }
}

/**
 * Keyboard model of the tab strip (WAI-ARIA tabs, automatic activation): left and right move
 * and wrap, Home and End jump. Any other key returns null so the handler leaves it alone.
 */
export function tabAfterKey(current: SettingsTab, key: string): SettingsTab | null {
  const index = SETTINGS_TABS.findIndex((tab) => tab.id === current)
  if (index < 0) return null
  switch (key) {
    case 'ArrowRight':
      return SETTINGS_TABS[(index + 1) % SETTINGS_TABS.length].id
    case 'ArrowLeft':
      return SETTINGS_TABS[(index - 1 + SETTINGS_TABS.length) % SETTINGS_TABS.length].id
    case 'Home':
      return SETTINGS_TABS[0].id
    case 'End':
      return SETTINGS_TABS[SETTINGS_TABS.length - 1].id
    default:
      return null
  }
}

// ── Sync > Beebeeb in Finder ────────────────────────────────────────────────

/** The mono reason line is one line of at most 30 characters (spec section 4), cut with an ellipsis. */
export const MONO_REASON_MAX = 30

export function monoReason(category: string | null | undefined): string | null {
  const trimmed = category?.trim()
  if (!trimmed) return null
  const line = `reason: ${trimmed}`
  return line.length <= MONO_REASON_MAX ? line : `${line.slice(0, MONO_REASON_MAX - 1)}…`
}

/**
 * What a failed Add to Finder says. Spec section 4 (state f2) words exactly one case, the timeout
 * from screenshot 1: "macOS didn't respond in time." with the mono line `reason: timeout`, and
 * no remedy advice (no task owns the cause). Every other category keeps the mono line (the
 * 2026-09-30 amendment: `reason: <category>` for every category) and gets one neutral sentence
 * that claims nothing about the cause. The raw error text is never shown: it names internals
 * ("File Provider domain") and it already goes into the support bundle.
 */
export const FINDER_FAILURE_TITLE = 'Couldn’t add Beebeeb to Finder'

export function finderFailureCopy(category: string | null | undefined): { sentence: string; reason: string | null } {
  return {
    sentence: category === 'timeout' ? 'macOS didn’t respond in time.' : 'macOS couldn’t finish setting it up.',
    reason: monoReason(category),
  }
}

export type FinderRow =
  | { kind: 'loading' }
  | { kind: 'unavailable' }
  | { kind: 'adding' }
  | { kind: 'added' }
  | { kind: 'missing' }
  | { kind: 'failed'; title: string; sentence: string; reason: string | null }
  | { kind: 'user_disabled'; message: string }

/**
 * One row, one state. `loadFailed` is a LOAD failure (the state could not be read at all); a
 * failed install is a state the backend returns (decision D1: saved and returned as `Ok`, never
 * also thrown), so it arrives here as `state.last_error` and is shown once, under the row.
 */
export function finderRow(state: FinderInstallState | null, attempting: boolean, loadFailed = false): FinderRow {
  if (attempting) return { kind: 'adding' }
  if (state === null) return loadFailed ? { kind: 'unavailable' } : { kind: 'loading' }
  if (state.installed) return { kind: 'added' }
  const notice = finderInstallNotice(state)
  if (notice?.kind === 'user_disabled') return { kind: 'user_disabled', message: notice.message }
  if (notice?.kind === 'error' || state.status === 'error') {
    return { kind: 'failed', title: FINDER_FAILURE_TITLE, ...finderFailureCopy(state.reason_category) }
  }
  return { kind: 'missing' }
}

/**
 * The hint under "Beebeeb in Finder". It may say the vault appears in Finder only while it does;
 * before that it says what adding does (the words of spec state f). While the state is still
 * loading it says nothing rather than guess.
 */
export function finderHint(row: FinderRow): string {
  switch (row.kind) {
    case 'added':
      return 'Your vault appears under Locations in Finder.'
    case 'loading':
      return ''
    case 'unavailable':
      return 'Couldn’t check Finder.'
    default:
      return 'Add it to see your files in Finder like any other folder.'
  }
}

/** What the Repair confirmation says. It must name the login item (spec section 5). */
export const REPAIR_TITLE = 'Repair Beebeeb in Finder?'
export const REPAIR_BODY =
  'Beebeeb removes its Finder location and turns off Open Beebeeb at login. Files waiting to upload are kept. You can add it back afterwards.'

/** After a successful repair: one neutral line, or nothing when there is nothing to add. */
export function repairNote(result: { pending_operations_preserved: number; warnings: string[] }): string | null {
  const parts: string[] = []
  const kept = result.pending_operations_preserved
  if (kept > 0) parts.push(`${kept} ${kept === 1 ? 'change' : 'changes'} waiting to upload ${kept === 1 ? 'was' : 'were'} kept.`)
  for (const warning of result.warnings) {
    const text = warning.trim()
    if (text) parts.push(text)
  }
  return parts.length > 0 ? parts.join(' ') : null
}

/**
 * Task 1882 (P0, spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md §5): the one
 * sentence shown when removing Beebeeb from Finder kept files that had not reached the server.
 * The folder's path follows it, in mono. The same string is `PRESERVED_FILES_SENTENCE` in
 * src-tauri/src/finder_removal.rs, which the app's alert shows after a sign-out;
 * tests/finderPreservedFiles.test.ts pins the two equal.
 */
export const PRESERVED_FILES_SENTENCE = 'Files that hadn’t reached your vault yet were kept on this Mac, in this folder:'

/**
 * Round 3 (re-review D3): the sentence on the saved kept-folder row in Settings › Sync. The row
 * outlives the sign-out that kept the files, so after an account switch another account reads it:
 * it says nothing about "your vault", and names no account or provider. The alert and the Repair
 * line appear straight after the removal and keep `PRESERVED_FILES_SENTENCE`.
 */
export const KEPT_FOLDER_ROW_SENTENCE = 'Files that had not reached the server were kept in this folder:'

/**
 * 1882 r4: the code a failed Repair's error starts with when Repair failed AFTER it removed the
 * Finder location. The same string is `REPAIR_FAILED_AFTER_REMOVAL_CODE` in
 * src-tauri/src/finder_removal.rs; tests/finderPreservedFiles.test.ts pins the two equal.
 */
export const REPAIR_FAILED_AFTER_REMOVAL_CODE = 'repair_failed_after_removal'

/** The title of the note a Repair that failed after the removal shows (1882 r4, lead ruling). */
export const REPAIR_REMOVED_TITLE = 'Beebeeb was removed from Finder'

/**
 * 1882 r4: when Repair fails AFTER it removed the Finder location, "Nothing was changed that you
 * need to undo" is false. Rust marks that failure with a code at the start of its error; this turns
 * it into the note to show: Beebeeb is gone from Finder, and the button that adds it back
 * (`button` is that button's own name on the surface that shows the note). The raw detail is never
 * shown. Any other error, a failure before the removal, is not this note: `null`.
 */
export function repairRemovedNotice(reason: string, button: string): { title: string; body: string } | null {
  const after = reason === REPAIR_FAILED_AFTER_REMOVAL_CODE || reason.startsWith(`${REPAIR_FAILED_AFTER_REMOVAL_CODE}:`)
  return after ? { title: REPAIR_REMOVED_TITLE, body: repairRemovedBody(button) } : null
}

/** The sentence under that title; `button` is the name of the button that adds Beebeeb back. */
export function repairRemovedBody(button: string): string {
  return `Repair couldn’t finish, so Beebeeb is no longer in Finder. Choose ${button} to add it back.`
}

/** The kept-files note for a removal's result: the sentence and the exact folder, or null when macOS kept nothing. */
export function preservedFilesNote(result: { preserved_location?: string | null }): { sentence: string; path: string } | null {
  const path = result.preserved_location
  if (typeof path !== 'string' || path.trim() === '') return null
  return { sentence: PRESERVED_FILES_SENTENCE, path }
}

/** The same note as one line, for a surface without a mono line of its own (the compact window). */
export function preservedFilesLine(result: { preserved_location?: string | null }): string | null {
  const note = preservedFilesNote(result)
  return note ? `${note.sentence} ${note.path}` : null
}

/**
 * What the `dismiss_kept_unsynced_folder` command reports (1882 r5): `cleared` is whether the saved
 * folder was the one the row showed and is now gone; `current` is the folder saved after the
 * command, null when none is. The same two keys as `DismissOutcome` in `finder_removal.rs`.
 */
export interface DismissKeptFolderResult {
  cleared: boolean
  current: string | null
}

/**
 * The folder the row shows after a Dismiss: none when Rust cleared it, otherwise the folder saved
 * now. A row that showed an older folder when a newer one was kept is not cleared, so it shows the
 * newer one instead of vanishing as if the person had dismissed it.
 */
export function keptFolderAfterDismiss(result: DismissKeptFolderResult): string | null {
  if (result.cleared) return null
  return preservedFilesNote({ preserved_location: result.current })?.path ?? null
}

// ── Sync > Keep on this Mac ─────────────────────────────────────────────────

export interface FolderEntry {
  id: string
  name: string
  /** Where it sits in the vault, shown as the second line. Empty at the vault root. */
  where: string
  pinned: boolean
}

function walk(items: VaultItem[], parents: string[], out: FolderEntry[], sawPinField: { value: boolean }): void {
  for (const item of items) {
    if (!item.is_folder) continue
    if (typeof item.pinned === 'boolean') sawPinField.value = true
    out.push({ id: item.id, name: item.name, where: parents.join(' / '), pinned: item.pinned === true })
    walk(item.children ?? [], [...parents, item.name], out, sawPinField)
  }
}

export type KeepOnMac =
  | { kind: 'loading' }
  | { kind: 'failed' }
  /** Folders exist but the backend sent no pin state for any of them: the count would be a guess. */
  | { kind: 'unreported' }
  | { kind: 'ready'; folders: FolderEntry[]; pinned: number }

/**
 * Reads `list_remote_tree` for the Keep on this Mac row. It never says "0 folders" when the
 * backend simply does not report pin state (today's `list_vault_folders` carries `excluded`, not
 * `pinned`; see decisions/1683-s4-undrawn-and-unbacked.md): that case is `unreported`.
 */
export function keepOnMac(tree: VaultItem[] | null, loadFailed: boolean): KeepOnMac {
  if (tree === null) return loadFailed ? { kind: 'failed' } : { kind: 'loading' }
  const folders: FolderEntry[] = []
  const sawPinField = { value: false }
  walk(tree, [], folders, sawPinField)
  if (folders.length > 0 && !sawPinField.value) return { kind: 'unreported' }
  return { kind: 'ready', folders, pinned: folders.filter((folder) => folder.pinned).length }
}

export function keepCountLabel(pinned: number): string {
  if (pinned === 0) return 'No folders'
  return pinned === 1 ? '1 folder' : `${pinned} folders`
}

// ── Sync > Speed ────────────────────────────────────────────────────────────

/** 0 means no limit, like the desktop config. Decimal Mbps, like the CLI and the old slider. */
export const SPEED_PRESETS_KBPS: readonly number[] = [0, 1000, 2000, 5000, 10000, 20000, 50000, 100000]

export function speedLabel(kbps: number): string {
  if (!(kbps > 0)) return 'No limit'
  if (kbps >= 1000) return `${Number((kbps / 1000).toFixed(1))} Mbps`
  return `${Math.round(kbps)} kbps`
}

/** The presets, plus the saved value when an older version of the app saved one that is not a preset. */
export function speedOptions(current: number): Array<{ value: number; label: string }> {
  const values = [...SPEED_PRESETS_KBPS]
  const saved = Number.isFinite(current) && current > 0 ? Math.round(current) : 0
  if (!values.includes(saved)) values.push(saved)
  values.sort((a, b) => a - b)
  return values.map((value) => ({ value, label: speedLabel(value) }))
}

// ── Account ─────────────────────────────────────────────────────────────────

export function accountInitial(email: string | null | undefined): string {
  const letter = email?.trim().match(/[\p{L}\p{N}]/u)?.[0]
  return letter ? letter.toUpperCase() : 'B'
}

/** `Basic plan · renews 14 Oct 2026`, or just the plan when nothing is due. Never invents a date. */
export function planLine(sub: Pick<Subscription, 'plan' | 'status' | 'current_period_end'> | null | undefined): string | null {
  if (!sub?.plan) return null
  const plan = `${titleCasePlan(sub.plan)} plan`
  if (!sub.current_period_end) return plan
  const date = planRenewalCopy(sub.current_period_end).split(' · ')[0]
  if (!date || date.startsWith('No renewal')) return plan
  const verb = ['active', 'trialing'].includes(sub.status.toLowerCase()) ? 'renews' : 'ends'
  return `${plan} · ${verb} ${date}`
}

/** The storage red threshold, spec section 3: neutral, and red from 90 % up. */
export const STORAGE_RED_PERCENT = 90

export function storageLine(
  storage: { used_bytes: number; quota_bytes: number } | null | undefined,
  locale?: string,
): { label: string; percent: number; red: boolean } | null {
  if (!storage || !(storage.quota_bytes > 0)) return null
  const { used, quota } = formatStorageUsage(storage.used_bytes, storage.quota_bytes, locale)
  const percent = quotaPercent(storage.used_bytes, storage.quota_bytes)
  return { label: `${used} of ${quota}`, percent, red: percent >= STORAGE_RED_PERCENT }
}

// ── About ───────────────────────────────────────────────────────────────────

/**
 * The support page. The same address the Help menu already opens for "get help"
 * (`KEYBOARD_SHORTCUTS_URL` in src-tauri/src/lib.rs, which explains why /support and not /faq).
 */
export const HELP_URL = 'https://beebeeb.io/support'

export interface UpdateRow {
  hint: string
  action: 'check' | 'checking' | 'install'
}

/**
 * The About row answers its own button: the result is the row's hint, inline, because this
 * window does not mount the update toast (`ManualUpdateFeedback`), so there is one surface.
 */
export function updateRow(state: ManualUpdateCheckState, version: string | null): UpdateRow {
  const current = version ? `Version ${version}` : ''
  switch (state.kind) {
    case 'idle':
      return { hint: current, action: 'check' }
    case 'checking':
      return { hint: 'Checking…', action: 'checking' }
    case 'up_to_date':
      return { hint: `Version ${state.currentVersion} is the latest.`, action: 'check' }
    case 'update_available':
      return { hint: `Version ${state.version} is available.`, action: 'install' }
    case 'downgrade_available':
      return { hint: `${current}${current ? ' · ' : ''}The ${state.channel} channel is at ${state.version}.`, action: 'check' }
    case 'error':
      return { hint: 'Couldn’t check for updates. Try again in a moment.', action: 'check' }
  }
}

// ── General ─────────────────────────────────────────────────────────────────

export type NotificationKey = 'notify_conflicts' | 'notify_sync_complete' | 'notify_quota_warnings'

/**
 * "Tell me about": the three preferences the desktop config already has (keys unchanged), in the
 * plain words of spec section 5. "Local cache almost full" is the same setting the old page
 * called "Local cache warnings"; it is about this Mac's cache limit, not its disk.
 */
export const NOTIFICATION_ROWS: ReadonlyArray<{ key: NotificationKey; label: string; hint?: string }> = [
  { key: 'notify_conflicts', label: 'Conflicts', hint: 'When two versions of a file need a decision.' },
  { key: 'notify_sync_complete', label: 'Sync finished' },
  { key: 'notify_quota_warnings', label: 'Local cache almost full' },
]
