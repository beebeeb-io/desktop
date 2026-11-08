// B14 fixtures (account settings): the board's example account, Anna. A plain browser (headless frames,
// `bun run dev`) has no IPC bridge, so these stand in for the commands. Never real data.

export type ProductState = 'This app' | 'Signed in' | 'Not signed in'
export interface ProductRow { name: string; state: ProductState }

export interface DeviceGroup {
  /** One session id per Beebeeb app signed in on the device (a row's Sign out ends all of them). */
  session_ids: string[]
  name: string
  os: string
  apps: string[]
  /** RFC 3339, or null when the server has none. */
  last_active_at: string | null
  country_code: string | null
  is_current: boolean
}

export interface Passkey { id: string; name: string; created_at: string; last_used_at: string | null; prf: boolean }

export interface ActivityEvent { id: string; what: string; at: string; country: string | null; address: string | null }

export interface Factor { key: string; satisfied: boolean }

/** Fixed "now" so relative times in the frames never drift. */
export const FIXTURE_NOW = Date.parse('2026-11-05T19:42:00Z')

export const FIXTURE_ACCOUNT = {
  name: 'Anna de Vries',
  email: 'anna@example.eu',
  email_verified: true,
  password_changed_at: '2026-08-02T10:00:00Z',
  totp: { on: true, added_at: '2026-03-14T09:12:00Z', last_used_at: '2026-11-04T08:30:00Z', codes_left: 6, codes_total: 8 },
  activity_logging: true,
}

export const FIXTURE_PRODUCTS: ProductRow[] = [
  { name: 'Drive', state: 'This app' },
  { name: 'Pass', state: 'Signed in' },
  { name: 'VPN', state: 'Not signed in' },
  { name: 'Chat', state: 'Not signed in' },
  { name: 'Mail', state: 'Signed in' },
]

export const FIXTURE_DEVICES: DeviceGroup[] = [
  { session_ids: ['s-pc-1', 's-pc-2'], name: 'Anna’s ThinkPad', os: 'Windows 11', apps: ['Drive', 'Pass'], last_active_at: '2026-11-05T19:42:00Z', country_code: 'NL', is_current: true },
  { session_ids: ['s-ip-1', 's-ip-2'], name: 'Anna’s iPhone', os: 'iOS 19', apps: ['Drive', 'Pass'], last_active_at: '2026-11-05T19:30:00Z', country_code: 'NL', is_current: false },
  { session_ids: ['s-web-1'], name: 'Firefox on Linux', os: 'Web', apps: ['Drive'], last_active_at: '2026-11-03T14:05:00Z', country_code: 'DE', is_current: false },
  { session_ids: ['s-cli-1'], name: 'bb on workstation', os: 'Command line', apps: ['Drive'], last_active_at: '2026-10-31T11:20:00Z', country_code: 'FI', is_current: false },
]

export const FIXTURE_PASSKEYS: Passkey[] = [
  { id: 'pk-1', name: 'iCloud Keychain', created_at: '2026-04-02T18:00:00Z', last_used_at: '2026-11-01T07:50:00Z', prf: true },
  { id: 'pk-2', name: 'Windows Hello', created_at: '2026-06-21T12:10:00Z', last_used_at: '2026-11-05T08:02:00Z', prf: false },
]

export const FIXTURE_FACTORS: Factor[] = [
  { key: 'email_verified', satisfied: true },
  { key: 'two_factor_enabled', satisfied: true },
  { key: 'strong_password', satisfied: true },
  { key: 'recovery_phrase_saved', satisfied: true },
  { key: 'passkey_added', satisfied: true },
  { key: 'password_not_breached', satisfied: true },
  { key: 'all_devices_recognized', satisfied: true },
]

export const FIXTURE_ACTIVITY: ActivityEvent[] = [
  { id: 'e1', what: 'Signed in on Windows 11 · Drive', at: '2026-11-05T19:30:00Z', country: 'Netherlands', address: '145.53.12.7' },
  { id: 'e2', what: 'Two-factor code used', at: '2026-11-05T19:30:00Z', country: 'Netherlands', address: '145.53.12.7' },
  { id: 'e3', what: 'Signed in on iOS 19 · Pass', at: '2026-11-04T08:30:00Z', country: 'Netherlands', address: '84.241.200.18' },
  { id: 'e4', what: 'Passkey added: Windows Hello', at: '2026-11-02T12:10:00Z', country: 'Netherlands', address: '145.53.12.7' },
  { id: 'e5', what: 'Signed in on the web · Drive', at: '2026-10-28T14:05:00Z', country: 'Germany', address: '91.64.33.201' },
  { id: 'e6', what: 'Password changed', at: '2026-10-20T10:00:00Z', country: 'Netherlands', address: '145.53.12.7' },
]

/** The board's example authenticator key (a documentation value, not a real secret) and its QR modules. */
export const FIXTURE_TOTP = {
  secret: 'JBSWY3DPEHPK3PXP',
  otpauth_uri: 'otpauth://totp/Beebeeb:anna%40example.eu?secret=JBSWY3DPEHPK3PXP&issuer=Beebeeb',
  qr_rows: [
    '1111111010111101111110110010101111111',
    '1000001011001101011011111111001000001',
    '1011101001110000001000110100101011101',
    '1011101010100010110000101111001011101',
    '1011101001000101011100001010101011101',
    '1000001000100011001111101100001000001',
    '1111111010101010101010101010101111111',
    '0000000010010101011010110101100000000',
    '1011011101101100111101001011001001011',
    '0000010100001101101110110001111101010',
    '0011111100100101010001111000001100100',
    '1111010000011101000110100111111101100',
    '0100001001100110110000111111001011111',
    '0100100101000011110100001011011010001',
    '1110101010101110001100000110110110110',
    '1001000001110110100001111001010010010',
    '0101101110001001101111100000000110110',
    '1110010101001110110010111111010001111',
    '0010011011111001100011001011010010011',
    '0001010000101010010000101001001011001',
    '0001101001000010011111101010010011011',
    '1010000110101110001111011000110001000',
    '0011111001100000100000110010011011000',
    '1110100001110100000110010100010111110',
    '0101011110011001001000101011101000111',
    '0101010010101100110100100000101111011',
    '0100101110111111101100001000110110110',
    '1000100101011010100101110001001011000',
    '0010001101101001100011000100111111100',
    '0000000011000111010010011001100011101',
    '1111111011110111110000000110101010011',
    '1000001011100000100010011011100011001',
    '1011101001001100001101100010111111000',
    '1011101010100011011011101000111010110',
    '1011101011001100101000110001111111010',
    '1000001000111110100110101110000101100',
    '1111111011110111010110001111010101011',
  ],
}

export const FIXTURE_BACKUP_CODES = ['48201937', '75530218', '19087346', '63318052', '90472615', '28856130', '51744809', '36620974']

/** DW-A82: 72 hours after the link was used. */
export const FIXTURE_SHRED_AFTER = '2026-11-08T19:42:00Z'
