/**
 * Pure logic behind the macOS Settings window (task 1683 slice 4; spec
 * `docs/specs/2026-09-30-macos-menubar-popover.md` section 5, artboards j/k/l/m).
 *
 * Nothing here touches React or Tauri, so `tests/macSettingsModel.test.ts` can pin every
 * decision the window makes about what to say: which row shows which state, which sentence
 * a Finder failure gets, how a speed limit is written, what the plan line reads.
 *
 * Its section bodies (`MacSettings.tsx`) are hosted by the Beebeeb window's Settings pane (`MacApp.tsx`, task 1683
 * slice 1, two-surface spec §7.8).
 */
import type { DesktopTheme, EngineRefusal, SpeedtestResult, StorageBreakdown, Subscription, VaultItem } from './desktopApi'
import type { FinderFailureReason, FinderSetupView } from './finderSetup'
import { ACCOUNT_SETTINGS_URL, RELEASE_CHANNELS, type ReleaseChannelId } from './settingsConstants'
import {
  FINDER_ADDING_LINE,
  FINDER_READY_LINE,
  FINDER_RESTING_LINE,
  FINDER_UNAVAILABLE_LINE,
  finderRepairWarningNote,
  finderSetupPresentation,
  type FinderSetupAction,
} from './finderSetupCopy'
import { quotaPercent, titleCasePlan, planRenewalCopy } from './planPresentation'
import type { ManualUpdateCheckState } from './windows/updateCheckViewModel'
import { updateAvailableLine } from './macUpdateCopy'
import { updateOffer } from './macUpdateOffer'
import { formatStorageSize, formatStorageUsage } from './storageFormat'
import { normalizeThemePreference } from './windows/advancedSettingsModel'

// ── Tabs ────────────────────────────────────────────────────────────────────

export const SETTINGS_TABS = [
  { id: 'general', label: 'General', icon: 'settings' },
  { id: 'account', label: 'Account', icon: 'users' },
  { id: 'sync', label: 'Sync', icon: 'refresh' },
  { id: 'security', label: 'Security', icon: 'shield' },
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
 * Keyboard model of the Settings list (task 1683 slice 1, two-surface spec §7.8 and §11): the shipped tab strip's
 * automatic activation, with ↑ and ↓ in place of ← and → because the tabs became the sidebar list (ruling 1). Down
 * and up move and wrap, Home and End jump, over `order` (the list as drawn; the tab order by default). Any other key
 * returns null so the handler leaves it alone.
 */
export function tabAfterKey(
  current: SettingsTab,
  key: string,
  order: readonly SettingsTab[] = SETTINGS_TABS.map((tab) => tab.id),
): SettingsTab | null {
  return listAfterKey(order, current, key)
}

/** The same keys over any vertical list: the Beebeeb window's own sidebar uses them too (two-surface spec §7.6). */
export function listAfterKey<T>(list: readonly T[], current: T, key: string): T | null {
  const index = list.indexOf(current)
  if (index < 0) return null
  switch (key) {
    case 'ArrowDown':
      return list[(index + 1) % list.length]
    case 'ArrowUp':
      return list[(index - 1 + list.length) % list.length]
    case 'Home':
      return list[0]
    case 'End':
      return list[list.length - 1]
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

export type FinderRow =
  | { kind: 'loading' }
  | { kind: 'unavailable' }
  | { kind: 'adding' }
  /** A loaded `Missing` (FA-I2): its one sentence, no button, no activity. */
  | { kind: 'resting' }
  | { kind: 'added' }
  | {
      kind: 'notice'
      tone: 'alert' | 'status'
      reason: FinderFailureReason
      sentence: string
      /** `null` for an unconfirmed engine stop: only a relaunch helps (must-render row 9). */
      action: FinderSetupAction | null
      actionLabel: string | null
    }

/**
 * One row, one reconciler state (spec 2026-10-06). `loadFailed` is a LOAD failure (the state could
 * not be read at all); a failed setup is part of the state and is shown once, under the row. A loaded
 * `Missing` is a resting state (after a sign-out, a Lock, or with no keys on this Mac; spec §5.2 as
 * amended, lead ruling FA-I2): a quiet row with one sentence and no button, never "Adding", and there
 * is no "missing" row with an add button (R5). The sentence and the action of a notice come from
 * `finderSetupCopy` (one sentence, one action per reason); the raw error never reaches this row.
 * `refusal` (`sync_status.engine_refusal`) turns a `failed` + `unknown` into the refusal's sentence.
 */
export function finderRow(view: FinderSetupView | null, loadFailed = false, refusal: EngineRefusal | null = null): FinderRow {
  if (view === null) return loadFailed ? { kind: 'unavailable' } : { kind: 'loading' }
  const presentation = finderSetupPresentation(view, refusal)
  switch (presentation.kind) {
    case 'ready':
      return { kind: 'added' }
    case 'adding':
      return { kind: 'adding' }
    case 'notice': {
      const { tone, reason, sentence, action, actionLabel } = presentation
      return { kind: 'notice', tone, reason, sentence, action, actionLabel }
    }
    case 'resting':
      return { kind: 'resting' }
    default:
      return { kind: 'loading' }
  }
}

/** The hint under "Beebeeb in Finder". A notice row says its sentence in the Note, not here. */
export function finderHint(row: FinderRow): string {
  switch (row.kind) {
    case 'added':
      return FINDER_READY_LINE
    case 'adding':
      return FINDER_ADDING_LINE
    case 'resting':
      return FINDER_RESTING_LINE
    case 'unavailable':
      return FINDER_UNAVAILABLE_LINE
    default:
      return ''
  }
}

/** What the Repair confirmation says. It must name the login item (spec section 5). */
export const REPAIR_TITLE = 'Repair Beebeeb in Finder?'
export const REPAIR_BODY =
  'Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.'

/**
 * The same confirmation on the Windows and Linux Finder page (`pages/SyncFolder.tsx`). It is not `REPAIR_BODY`: a Mac's
 * words are "Open Beebeeb at login" and "adds itself back", and neither is what the reset does there (PR #140 review, P3).
 * The old sentence's "stale local socket" is the one developer word it loses.
 */
export const RESET_FINDER_TITLE = 'Reset Finder integration?'
export const RESET_FINDER_BODY = 'Beebeeb will remove its Finder location and turn off Start at login. Files waiting to upload are kept.'

/**
 * After a successful repair: one neutral line, or nothing when there is nothing to add. A repair's
 * `warnings` hold a bridge error code and a cache-file path (task 17b, fix round 1), so they are never
 * shown: any warning makes this one fixed sentence instead of the count.
 */
export function repairNote(result: { pending_operations_preserved: number; warnings: string[]; engine_stop_unconfirmed?: boolean }): string | null {
  const warned = finderRepairWarningNote(result)
  if (warned) return warned
  const kept = result.pending_operations_preserved
  return kept > 0 ? `${kept} ${kept === 1 ? 'change' : 'changes'} waiting to upload ${kept === 1 ? 'was' : 'were'} kept.` : null
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

/** `Basic plan`: the short line under the account in the Beebeeb window's sidebar (two-surface hi-fi, section 4). */
export function planShortLine(sub: Pick<Subscription, 'plan'> | null | undefined): string | null {
  return sub?.plan ? `${titleCasePlan(sub.plan)} plan` : null
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

export { ACCOUNT_SETTINGS_URL }

// ── Account > Storage details ───────────────────────────────────────────────

/** Spec §10.2 #30: the Insights subtitle, without the PC and without a claim about what is encrypted where. */
export const STORAGE_DETAILS_SUBTITLE = 'Where your storage goes, by type and by your largest files.'

/** How many of the largest files the sheet lists. */
export const STORAGE_DETAILS_LARGEST = 10

const CATEGORY_ORDER = ['media', 'documents', 'other'] as const
const CATEGORY_LABEL = { media: 'Media', documents: 'Documents', other: 'Other' } as const

function fileCountLabel(count: number, locale?: string): string {
  let formatted: string
  try {
    formatted = new Intl.NumberFormat(locale).format(count)
  } catch {
    formatted = new Intl.NumberFormat('en').format(count)
  }
  return `${formatted} ${count === 1 ? 'file' : 'files'}`
}

export interface StorageDetails {
  /** Nothing is stored yet: the sheet says so instead of drawing a table of zeros. */
  empty: boolean
  /** `1.291 files · 84,3 GB` */
  total: string
  types: Array<{ key: (typeof CATEGORY_ORDER)[number]; label: string; size: string; files: string }>
  largest: Array<{ id: string; path: string; hint: string }>
}

/** The Insights content (by type, largest files) in a Mac's words and number format. */
export function storageDetails(breakdown: StorageBreakdown, locale?: string): StorageDetails {
  const byKey = new Map(breakdown.by_category.map((row) => [row.category, row]))
  return {
    empty: breakdown.total_files === 0 && breakdown.total_bytes === 0,
    total: `${fileCountLabel(breakdown.total_files, locale)} · ${formatStorageSize(breakdown.total_bytes, locale)}`,
    types: CATEGORY_ORDER.map((key) => {
      const row = byKey.get(key)
      return { key, label: CATEGORY_LABEL[key], size: formatStorageSize(row?.bytes ?? 0, locale), files: fileCountLabel(row?.file_count ?? 0, locale) }
    }),
    largest: [...breakdown.largest_files]
      .sort((a, b) => b.size_bytes - a.size_bytes)
      .slice(0, STORAGE_DETAILS_LARGEST)
      .map((file) => ({ id: file.file_id, path: file.path, hint: `${CATEGORY_LABEL[file.category]} · ${formatStorageSize(file.size_bytes, locale)}` })),
  }
}

// ── About ───────────────────────────────────────────────────────────────────

/**
 * The support page. The same address the Help menu already opens for "get help"
 * (`KEYBOARD_SHORTCUTS_URL` in src-tauri/src/lib.rs, which explains why /support and not /faq).
 */
export const HELP_URL = 'https://beebeeb.io/support'

export interface UpdateRow {
  hint: string
  action: 'check' | 'checking' | 'download'
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
      // Task 2176: a Mac downloads an update and never installs it in place (`macUpdateCopy.ts` says why).
      return { hint: `${updateAvailableLine(state.version)} ${updateOffer(state.channel, state.version).hint}`, action: 'download' }
    case 'downgrade_available':
      // The offer has its own row (`downgradeRow`), so this row keeps to the version this Mac runs.
      return { hint: current, action: 'check' }
    case 'error':
      return { hint: 'Couldn’t check for updates. Try again in a moment.', action: 'check' }
  }
}

export type { ReleaseChannelId }

export { RELEASE_CHANNELS }

/** A saved channel this app does not know reads as Stable, as the updater does. */
export function releaseChannelOf(value: unknown): ReleaseChannelId {
  return RELEASE_CHANNELS.find((channel) => channel.value === value)?.value ?? 'stable'
}

const channelLabel = (id: ReleaseChannelId) => RELEASE_CHANNELS.find((channel) => channel.value === id)?.label ?? 'Stable'

/**
 * The row for a selected channel that carries an older version than the one running. It informs and links; it offers no
 * install, because nothing on a Mac installs in place (task 2176). Null for every other state.
 */
export function downgradeRow(state: ManualUpdateCheckState): { label: string; hint: string } | null {
  if (state.kind !== 'downgrade_available') return null
  return {
    label: `The ${channelLabel(state.channel)} channel is at version ${state.version}`,
    hint: `This Mac runs ${state.currentVersion}, from the ${channelLabel(state.currentChannel)} channel. Read the release notes before you go back to an older version.`,
  }
}

/** What the speed test does, in plain words (spec §10.2 #26: no "parity with the CLI"). */
export const SPEED_TEST_HINT = 'Measures how fast this Mac reaches Beebeeb: 5 pings, then 3 uploads and 3 downloads of 10 MB. Takes 30 to 90 seconds.'

/** What a run that measured nothing says: the host reports a failed direction as an error, and this is the same sentence for a result that still arrives without a speed (PR #140 review, P2-7). */
export const SPEED_TEST_NO_FIGURES = 'The test finished without a speed to report. Check your internet connection and try again.'

/** The plain reason a result cannot be shown, or null when it can: a direction that never moved a byte is no "0.00 Mbps". */
export function speedTestFailure(result: SpeedtestResult): string | null {
  const moved = (bytesPerSecond: number) => Number.isFinite(bytesPerSecond) && bytesPerSecond > 0
  return moved(result.upload_bps) && moved(result.download_bps) ? null : SPEED_TEST_NO_FIGURES
}

/** Bytes per second as megabits per second, decimal like the CLI and the old slider: two places under 10, one above. */
function megabits(bytesPerSecond: number): string {
  const mbps = (bytesPerSecond * 8) / 1_000_000
  if (mbps < 0.01) return '0.00'
  return mbps < 10 ? mbps.toFixed(2) : mbps.toFixed(1)
}

/** The result as one mono line: `Latency 24 ms (18 to 41) · Upload 35.2 Mbps · Download 94.0 Mbps`. */
export function speedTestSummary(result: SpeedtestResult): string {
  const { min_ms, avg_ms, max_ms } = result.latency
  return `Latency ${avg_ms} ms (${min_ms} to ${max_ms}) · Upload ${megabits(result.upload_bps)} Mbps · Download ${megabits(result.download_bps)} Mbps`
}

// ── General ─────────────────────────────────────────────────────────────────

export const APPEARANCE_OPTIONS: ReadonlyArray<{ value: DesktopTheme; label: string }> = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

/** A theme this app does not know reads as System. */
export function appearanceValue(theme: unknown): DesktopTheme {
  return normalizeThemePreference(theme)
}

/** The hint under Appearance: System follows macOS (spec §7.8), the other two say what they hold still. */
export function appearanceHint(theme: unknown): string {
  const value = appearanceValue(theme)
  return value === 'light' ? 'Always light.' : value === 'dark' ? 'Always dark.' : 'Follow macOS.'
}

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
