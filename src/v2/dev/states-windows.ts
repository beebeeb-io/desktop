// Dev only (batch M2): live state recipes for B09 (the flyout; File Explorer), B10 (sign-in, approvals, lock, no plan)
// and B11 (create an account, forgot password, install, first run, the guide). The sign-in states open in the main
// window signed out (AuthGate shows the account entry), the first-run states in the onboarding window signed in.
import type { Recipe } from './states'
import { SCENARIOS } from '../tray/fixtures'
import { QR_SVG, REQUEST_CODE, SECONDS_LEFT, THIS_PC } from '../source/fixtures/auth'
import { EMAIL, KNOWN_FOLDERS, METER_GOOD, METER_SHORT, METER_STRONG, POLICY, POSITIONS, RECOVER_PARTIAL, RECOVER_WRONG, WORDS } from '../source/fixtures/signup'

// ── B09: the flyout ─────────────────────────────────────────────────────────────────────────────────────────────
const flyout = (name: string, o: Recipe = {}): Recipe => ({ window: 'tray', ...o, reads: { popover_snapshot: SCENARIOS[name].snapshot, ...o.reads } })
const FLYOUT_EXTRAS = (what: string) => `the flyout’s ${what} comes from no live source: TrayApp derives its extras from drive_status, which does not carry it`
const EXPLORER = 'File Explorer, its menus and its toasts are drawn by Windows (the board draws them for reference)'

// ── B10: sign-in ────────────────────────────────────────────────────────────────────────────────────────────────
const OUT = { sync_status: { logged_in: false } }
const ANNA_EMAIL = 'anna@northwind.studio'
const signIn = (o: Recipe = {}): Recipe => ({ ...o, q: 'section=files', reads: { ...OUT, last_signed_in_email: ANNA_EMAIL, ...o.reads } })
const PASSWORD = 'correct horse battery'
const submit = (): NonNullable<Recipe['steps']> => [{ wait: 'field:Password' }, { fill: 'field:Password', value: PASSWORD }, { click: 'btn:Sign in' }]
const OUTCOME = { requires_2fa: false, reauthenticated: false, vault_unlocked: true, key_replaced: false, account_mismatch: null }
const device = (status: string, secondsLeft = SECONDS_LEFT) => ({ status, user_code: REQUEST_CODE, seconds_left: secondsLeft, device_name: THIS_PC, qr_svg: QR_SVG })
const toDevice = (poll?: unknown): Recipe => signIn({
  answers: { auth_device_request_start: [device('waiting')], ...(poll ? { auth_device_request_poll: [poll] } : {}) },
  steps: [{ click: 'btn:Sign in with another device' }, { pause: poll ? 1400 : 300 }],
})
const NO_CONTINUE_AS = 'Continue as needs the shared Beebeeb-apps session store (task 2323): SignInFlow never has an account to offer'
const NO_APPROVE = 'the approval screens (auth/Approve.tsx) are not mounted in the live window: Settings › Security’s Approve a sign-in has no screen to open'
const PHRASE = WORDS.join(' ')

// ── B11 ─────────────────────────────────────────────────────────────────────────────────────────────────────────
const create = (step: string, seeds: Record<string, unknown> = {}): Recipe =>
  signIn({ reads: { auth_signup_begin: POLICY, auth_signup_default_device_name: THIS_PC }, seeds: { 'entry.mode': { kind: 'create' }, 'create.step': step, ...seeds } })
const forgot = (screen: string, seeds: Record<string, unknown> = {}): Recipe =>
  signIn({ seeds: { 'entry.mode': { kind: 'forgot' }, 'forgot.screen': screen, ...seeds } })
const SIGNED_IN = { sync_status: { logged_in: true, vault_unlocked: true }, get_known_folder_backup: KNOWN_FOLDERS, autostart_enabled: true }
const finder = (setup: 'ready' | 'adding' | 'failed') => ({
  setup, reason: setup === 'failed' ? 'shell_registration' : null, launch_location: 'installed', attempt: 1, max_attempts: 3,
  last_failure: setup === 'failed' ? { code: 0x8007017c, at: 0 } : null,
})
const setup = (step: string, o: { finder?: 'ready' | 'adding' | 'failed' } = {}): Recipe => ({
  window: 'onboarding', reads: { ...SIGNED_IN, finder_setup_state: finder(o.finder ?? 'ready') }, seeds: { 'setup.step': step },
})
const NO_GUIDE = 'the first-time guide (guide/Guide.tsx) is not mounted in the live window: nothing starts it after setup or from Settings'

export const WINDOW_STATES: Record<string, Recipe> = {
  // ── B09: the notification area and its flyout ─────────────────────────────────────────────────────────────────
  'DW-1201': { window: 'tray', native: 'the taskbar and the notification area are drawn by Windows; the flyout opens above them' },
  'DW-1202': flyout('up-to-date'),
  'DW-1203': flyout('syncing', { q: 'scenario=syncing' }),
  'DW-1204': flyout('paused'),
  'DW-1205': flyout('needs-decision'),
  'DW-1206': flyout('offline'),
  'DW-1207': flyout('stopped', { q: 'scenario=problems' }),
  'DW-1208': flyout('almost-full'),
  'DW-1209': flyout('storage-full'),
  'DW-1210': flyout('update'),
  'DW-1211': flyout('locked'),
  'DW-1212': flyout('checking', { reads: { popover_snapshot: { error: 'not yet' } } }),
  'DW-1213': flyout('no-plan', { gap: FLYOUT_EXTRAS('no-plan state') }),
  'DW-1214': flyout('update-required', { gap: FLYOUT_EXTRAS('update-required state') }),
  'DW-1215': flyout('finder-adding'),
  'DW-1216': flyout('finder-failed'),
  'DW-1219': flyout('signed-out'),
  'DW-1220': flyout('last-50', { gap: 'the live flyout opens at the top of Recent changes; the board’s frame is scrolled to its end' }),
  'DW-1221': flyout('longest'),
  'DW-1222': flyout('metered', { gap: FLYOUT_EXTRAS('metered connection') }),
  'DW-1180': { window: 'tray', native: EXPLORER },
  'DW-1181': { window: 'tray', native: EXPLORER },
  'DW-1183': { window: 'tray', native: EXPLORER },
  'DW-1182': { window: 'tray', native: EXPLORER },

  // ── B10: first run, sign in ───────────────────────────────────────────────────────────────────────────────────
  'DW-A01': { q: 'section=files', reads: OUT },
  'DW-A02': signIn({ gap: NO_CONTINUE_AS }),
  'DW-A03': signIn(),
  'DW-A04': signIn({ gap: NO_CONTINUE_AS }),
  'DW-A05': signIn({ gap: NO_CONTINUE_AS }),
  'DW-A06': signIn({ gap: NO_CONTINUE_AS }),
  'DW-A07': toDevice(),
  'DW-A08': toDevice(device('signed_in')),
  'DW-A09': toDevice(device('denied')),
  'DW-A10': toDevice(device('expired', 0)),
  'DW-A11': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A12': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A13': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A14': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A15': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A16': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  'DW-A17': { q: 'section=settings&sub=security', gap: NO_APPROVE },
  // Email and password: each problem is the host's answer to the sign-in (desktop_login).
  'DW-A18': signIn({ answers: { desktop_login: [{ pending: true }] }, steps: submit() }),
  'DW-A19': signIn({ answers: { desktop_login: [{ error: 'invalid_credentials' }] }, steps: submit() }),
  'DW-A20': signIn({ answers: { desktop_login: [{ error: 'offline' }] }, steps: submit() }),
  'DW-A21': signIn({ answers: { desktop_login: [{ error: 'rate_limited:30' }] }, steps: submit() }),
  'DW-A22': signIn({ answers: { desktop_login: Array(5).fill({ error: 'invalid_credentials' }) }, steps: [...submit(), { click: 'btn:Sign in' }, { click: 'btn:Sign in' }, { click: 'btn:Sign in' }, { click: 'btn:Sign in' }] }),
  'DW-A63': signIn({ answers: { desktop_login: [{ error: 'account_deleted' }] }, steps: submit() }),
  'DW-A23': signIn({ answers: { desktop_login: [{ ...OUTCOME, vault_unlocked: false }] }, steps: submit() }),
  'DW-A24': signIn({
    answers: { desktop_login: [{ ...OUTCOME, vault_unlocked: false }], desktop_unlock_with_recovery_phrase: [{ error: 'wrong phrase' }], auth_phrase_unknown_words: [[2]] },
    steps: [...submit(), { fill: 'input', value: PHRASE }, { click: 'btn:Restore keys' }],
  }),
  'DW-A25': signIn({ answers: { desktop_login: [{ ...OUTCOME, vault_unlocked: false }] }, steps: [...submit(), { click: 'btn:I’ve lost my recovery phrase' }] }),
  'DW-A26': signIn({ answers: { desktop_login: [{ ...OUTCOME, requires_2fa: true }] }, steps: [...submit(), { fill: 'any:#totp', value: '4821' }] }),
  'DW-A27': signIn({ answers: { desktop_login: [{ ...OUTCOME, requires_2fa: true }], desktop_login_2fa: [{ error: 'invalid code' }] }, steps: [...submit(), { fill: 'any:#totp', value: '482193' }, { key: 'Enter', on: 'any:#totp' }] }),
  'DW-A28': signIn({ answers: { desktop_login: [{ ...OUTCOME, requires_2fa: true }] }, steps: [...submit(), { click: 'btn:Use a backup code' }] }),
  'DW-A29': signIn({ answers: { auth_passkey_sign_in: [{ pending: true }] }, steps: [{ click: 'btn:Use a passkey' }] }),
  'DW-A30': signIn({ answers: { auth_passkey_sign_in: [{ ...OUTCOME, vault_unlocked: false }] }, steps: [{ click: 'btn:Use a passkey' }] }),
  'DW-0009': { q: 'section=files', reads: { sync_status: { logged_in: true, vault_unlocked: false } } },
  'DW-0014': { q: 'section=files', reads: { sync_status: { logged_in: true, vault_unlocked: true }, account_subscription: { plan_state: 'none' } } },

  // ── B11: create an account ────────────────────────────────────────────────────────────────────────────────────
  'DW-A31': create('email'),
  'DW-A32': create('verify', { 'create.code': '4821' }),
  'DW-A33': create('verify', { 'create.code': '48210937', 'create.codeState': 'wrong' }),
  'DW-A34': create('verify', { 'create.notice': { tone: 'neutral', text: 'If this address can be used, a new email is on its way.' } }),
  'DW-A35': create('terms', { 'create.terms': { accepted: true, understood: false } }),
  'DW-A36': create('password', { 'create.pw': { password: 'Winter-harbour-lantern-42', confirm: 'Winter-harbour-lantern-42', show: false }, 'create.meter': METER_STRONG }),
  'DW-A37': create('password', { 'create.pw': { password: 'Winter-har', confirm: '', show: false }, 'create.meter': METER_SHORT }),
  'DW-A38': create('password', { 'create.pw': { password: 'Password123456789012345', confirm: '', show: false }, 'create.meter': METER_GOOD, 'create.breach': 'breached' }),
  'DW-A39': create('phrase', { 'create.words': WORDS }),
  'DW-A40': create('confirm', { 'create.positions': POSITIONS, 'create.answers': ['thank', 'worth', 'tha'], 'create.active': 2, 'create.suggestions': ['thank', 'that', 'theme'] }),
  'DW-A41': create('confirm', { 'create.positions': POSITIONS, 'create.answers': ['thank', 'wave', 'thank'], 'create.wrongWords': true }),
  'DW-A42': create('device'),
  'DW-A43': create('creating'),
  'DW-A44': create('done'),

  // ── B11: forgot password ──────────────────────────────────────────────────────────────────────────────────────
  'DW-A45': forgot('options'),
  'DW-A46': forgot('neither'),
  'DW-A47': forgot('phrase', { 'forgot.email': EMAIL, 'forgot.words': RECOVER_PARTIAL }),
  'DW-A48': forgot('phrase', { 'forgot.email': EMAIL, 'forgot.words': RECOVER_WRONG, 'forgot.unknown': [5], 'forgot.state': 'wrong' }),
  'DW-A49': forgot('password', { 'forgot.pw': { password: 'Winter-harbour-lantern-42', confirm: 'Winter-harbour-lantern-42', show: false }, 'forgot.meter': METER_STRONG }),
  'DW-A50': forgot('updated'),

  // ── B11: install, first run, the guide ────────────────────────────────────────────────────────────────────────
  'DW-1901': { window: 'onboarding', native: 'the NSIS installer is drawn by Windows (the board draws it for reference)' },
  'DW-1902': { window: 'onboarding', native: 'the NSIS installer is drawn by Windows (the board draws it for reference)' },
  'DW-0201': setup('passkey', { finder: 'adding' }),
  'DW-0202': setup('lock', { finder: 'adding' }),
  'DW-0203': setup('backup'),
  'DW-0204': setup('photos'),
  'DW-0206': setup('explorer'),
  'DW-0206b': setup('explorer', { finder: 'adding' }),
  'DW-0280': setup('explorer', { finder: 'failed' }),
  'DW-0207': setup('notifications'),
  'DW-0281': setup('login'),
  'DW-0208': setup('done'),
  'DW-0209': setup('compare'),
  'DW-0380': { q: 'section=files', gap: NO_GUIDE },
  'DW-0301': { q: 'section=files', gap: NO_GUIDE },
  'DW-0302': { q: 'section=files', gap: NO_GUIDE },
  'DW-0303': { q: 'section=shared', gap: NO_GUIDE },
  'DW-0304': { q: 'section=photos', gap: NO_GUIDE },
  'DW-0305': { q: 'section=files', gap: NO_GUIDE },
}

