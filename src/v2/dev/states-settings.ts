// Dev only (batch M2): live state recipes for B12 (the settings shell, Drive's sections, Backups), B13 (what the app
// does today) and B14 (account settings). The settings route opens the pane; a frame's variant of it (the props its
// frame passes, e.g. Bandwidth's metered variant) and its dialog are the pane's first state (seeds), the same strings
// the frames use, so the live pane is the real one with its real handlers.
import type { Recipe } from './states'
import { TEST_STATES } from '../source/fixtures/settings-today'
import { FIXTURE_BACKUP_PROBLEMS } from '../source/fixtures/settings'

const pane = (sub: string, o: { dialog?: string; props?: Record<string, unknown>; steps?: Recipe['steps']; gap?: string; native?: string; reads?: Recipe['reads'] } = {}): Recipe => {
  const seeds: Record<string, unknown> = {}
  if (o.dialog) seeds['settings.dialog'] = o.dialog
  if (o.props) seeds['settings.paneProps'] = o.props
  return {
    q: `section=settings&page=${sub}`, seeds, gap: o.gap, native: o.native, reads: o.reads,
    // Wait for the pane, so the seeds are read before they are cleared.
    steps: [{ wait: '[data-testid="settings-view"]' }, ...(o.steps ?? [])],
  }
}
const APP_LOCK = 'App lock has no setting and no Windows Hello step yet: the pane shows it off, and its switch and delay can’t be changed'
const security = (init: Record<string, unknown>) => pane('security', { props: { init } })
const account = (initial: string, codeDigits?: string) => pane('account', { props: { initial, codeDigits } })

export const SETTINGS_STATES: Record<string, Recipe> = {
  // ── B12: the settings shell ───────────────────────────────────────────────────────────────────────────────────
  'DW-0701': pane('general'),
  'DW-0781': pane('general', { props: { updateAvailable: '0.9.2' } }),
  // About says 0.9.1 is the newest (Back → General without a banner, DW-0751#5 → DW-0701); Check now finds 0.9.2
  // (DW-0751#1 → DW-0781): the updater's answer, a read, which General's banner then shows.
  'DW-0751': pane('about', { reads: {
    check_for_updates_now: { status: 'update_available', current_version: '0.9.1', channel: 'stable', version: '0.9.2', body: '', release_notes_url: 'https://beebeeb.io/releases' },
  } }),
  // Save and write email (DW-0782#0 → DW-0790b): report_problem's answer, the path it saved and whether the email opened.
  'DW-0782': pane('general', { dialog: 'report', reads: { report_problem: { path: '%LOCALAPPDATA%\\io.beebeeb.app\\diagnostics\\beebeeb-diagnostics-1793956320.json', email_opened: true } } }),
  // The plan dialog's Done returns to Account (DW-0708#1 → DW-A51): it is opened over Account.
  'DW-0708': pane('account', { dialog: 'plan' }),
  'DW-0712': pane('storage'),
  'DW-0713': pane('storage', { dialog: 'limit-previews' }),
  'DW-0714': pane('storage', { dialog: 'limit-downloads' }),
  // Free up space and Cancel return to Sync and File Explorer (DW-0780#0/#1 → DW-0746): opened from there (DW-0746#6).
  'DW-0780': pane('sync', { dialog: 'free-up' }),
  // App lock is stored nowhere and no command asks Windows Hello before it changes (blocked, M3e): the live pane shows
  // it off and unchangeable, never the board's lock that isn't there.
  'DW-0716': pane('security', { gap: APP_LOCK }),
  'DW-0716b': pane('security', { gap: APP_LOCK }),
  'DW-0750': pane('notifications'),
  'DW-0750b': pane('notifications', { props: { offInWindows: true } }),
  // ── B12: Drive's own sections ─────────────────────────────────────────────────────────────────────────────────
  'DW-0746': pane('sync'),
  'DW-0746b': pane('sync', { dialog: 'explorer-off' }),
  'DW-0744': pane('thumbnails'),
  'DW-0744b': pane('thumbnails', { dialog: 'apply:2K:HEIC' }),
  'DW-0745': pane('compare'),
  // ── B12: Backups on Windows ───────────────────────────────────────────────────────────────────────────────────
  'DW-0737': pane('backups'),
  'DW-1411': pane('backups', { props: { state: 'running' } }),
  'DW-0738': pane('backups', { dialog: 'picker', native: 'Windows draws the folder picker (the board draws it for reference)' }),
  'DW-1410': pane('backups', { dialog: 'stop:Projects' }),
  'DW-0739': pane('device'),
  'DW-0742': pane('device', { dialog: 'delete-backup' }),
  // The engine's list in a browser is the fixtures' (no backup problems there): the board's list is the pane's first state.
  'DW-0741': pane('backup-failures', { props: { problems: FIXTURE_BACKUP_PROBLEMS } }),

  // ── B13: bandwidth and the speed test ─────────────────────────────────────────────────────────────────────────
  'DW-0785': pane('bandwidth', { props: { hover: 44 } }),
  'DW-0785b': pane('bandwidth', { props: { variant: 'idle' } }),
  'DW-0785c': pane('bandwidth', { props: { variant: 'limited' } }),
  'DW-0785d': pane('bandwidth', { props: { scrollTo: 'limits', limitPopup: 'upload' } }),
  'DW-0785e': pane('bandwidth', { props: { variant: 'empty' } }),
  'DW-0785f': pane('bandwidth', { props: { variant: 'metered' } }),
  'DW-0786': pane('bandwidth', { props: { scrollTo: 'test', test: TEST_STATES.running } }),
  'DW-0786b': pane('bandwidth', { props: { scrollTo: 'test', test: TEST_STATES.downloads } }),
  'DW-0786c': pane('bandwidth', { props: { scrollTo: 'test', test: TEST_STATES.done } }),
  'DW-0786d': pane('bandwidth', { props: { scrollTo: 'test', test: TEST_STATES.error } }),
  // ── B13: recent notifications, storage insights, keep on this PC ──────────────────────────────────────────────
  'DW-0791': pane('recent'),
  'DW-0791b': pane('recent', { props: { empty: true } }),
  'DW-0783': pane('insights'),
  'DW-0783b': pane('insights', { props: { state: 'loading' } }),
  'DW-0783c': pane('insights', { props: { state: 'error' } }),
  'DW-0783d': pane('insights', { props: { state: 'empty' } }),
  'DW-0784': pane('sync', { dialog: 'keep:changed' }),
  'DW-0784b': pane('sync', { dialog: 'keep-confirm:changed' }),
  'DW-0784c': pane('sync', { dialog: 'keep:error' }),
  'DW-0784d': pane('sync', { dialog: 'keep:empty' }),
  // ── B13: general, storage location, about, diagnostics, sync ──────────────────────────────────────────────────
  'DW-0787': pane('general', { steps: [{ click: 'btn:Theme' }] }),
  'DW-0788': pane('location'),
  'DW-0788b': pane('location', { props: { chosen: 'de', toast: true } }),
  'DW-0788c': pane('location', { props: { chosen: 'fi', unavailable: true } }),
  'DW-0788d': pane('location', { props: { error: true } }),
  'DW-0789': pane('about', { steps: [{ click: 'btn:Release channel' }] }),
  'DW-0789b': pane('about', { dialog: 'release-notes' }),
  'DW-0790': pane('diagnostics'),
  'DW-0790b': pane('diagnostics', { props: { saved: true } }),
  'DW-0746c': pane('sync', { dialog: 'repair' }),
  'DW-0746d': pane('sync', { props: { repair: 'failed' } }),
  'DW-0746e': pane('sync', { props: { metered: true } }),

  // ── B14: account settings ─────────────────────────────────────────────────────────────────────────────────────
  'DW-A51': account('main'),
  'DW-A52': security({}),
  'DW-A84': security({ page: 'checkup' }),
  'DW-A84a': security({ page: 'checkup', scenario: 'no-passkey' }),
  'DW-A55': pane('devices'),
  'DW-A56': pane('devices', { props: { confirm: 'Anna’s iPhone' } }),
  'DW-A57': security({ dialog: 'signout' }),
  'DW-A57a': security({ dialog: 'signout-all' }),
  'DW-A64': account('name'),
  'DW-A65': account('email'),
  'DW-A66': account('email-code', '4829'),
  'DW-A67': security({ page: 'password' }),
  'DW-A68': security({ page: 'password', passwordError: true }),
  'DW-A69': security({ page: 'totp-setup', digits: '48' }),
  'DW-A70': security({ page: 'backup-codes' }),
  'DW-A71': security({ page: 'totp' }),
  'DW-A72': security({ page: 'totp', dialog: 'totp-off' }),
  'DW-A73': security({ page: 'passkeys' }),
  'DW-A74': security({ page: 'passkeys', scenario: 'no-passkeys-at-all' }),
  'DW-A75': security({ page: 'passkeys', passkeyMenu: 'iCloud Keychain' }),
  'DW-A76': security({ page: 'passkeys', removePasskey: 'iCloud Keychain' }),
  'DW-A77': security({ page: 'activity' }),
  'DW-A78': security({ page: 'activity', dialog: 'activity-off' }),
  'DW-A79': security({ page: 'activity', scenario: 'logging-off' }),
  'DW-A58': security({ page: 'delete-1' }),
  'DW-A80': security({ page: 'delete-2', digits: '71' }),
  'DW-A81': security({ page: 'delete-3' }),
  'DW-A82': security({ page: 'delete-4' }),
  'DW-A83': security({ page: 'delete-done' }),
}
