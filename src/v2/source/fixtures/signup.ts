// B11's fixtures: create an account, forgot password, first run, the guide. The phrase here is a public BIP39 test
// vector, never a real account's.
import type { KnownFolderStatus } from '../../../desktopApi'
import type { PasswordMeter, SignupPolicyView } from '../../auth/create/api'
import type { PreviewSample, SetupSummary } from '../../onboarding/views'

export const EMAIL = 'anna@example.eu'
export const POLICY: SignupPolicyView = { code_length: 8, resend_after_secs: 60, min_length: 12, verify_word_count: 3, terms_url: 'https://beebeeb.io/terms', privacy_url: 'https://beebeeb.io/privacy' }
export const WORDS = ['legal', 'winner', 'thank', 'year', 'wave', 'sausage', 'worth', 'useful', 'legal', 'winner', 'thank', 'yellow']
export const POSITIONS = [3, 7, 11]
export const DEVICE = 'Anna-Laptop'

export const METER_EMPTY: PasswordMeter = { length: 0, min_length: 12, missing: 12, strength: 'too_short', hint: 'none' }
export const METER_SHORT: PasswordMeter = { length: 10, min_length: 12, missing: 2, strength: 'too_short', hint: 'more' }
export const METER_GOOD: PasswordMeter = { length: 22, min_length: 12, missing: 0, strength: 'good', hint: 'mix_case' }
export const METER_STRONG: PasswordMeter = { length: 24, min_length: 12, missing: 0, strength: 'strong', hint: 'none' }

export const RECOVER_PARTIAL = ['legal', 'winner', 'thank', 'year', 'wave', 'sausage', 'worth', 'useful', '', '', '', '']
export const RECOVER_WRONG = ['legal', 'winner', 'thank', 'year', 'wave', 'sausige', 'worth', 'useful', 'legal', 'winner', 'thank', 'yellow']

const kf = (key: string, display_name: string, item_count: number, enabled = false): KnownFolderStatus =>
  ({ key, display_name, source_path: `C:\\Users\\anna\\${display_name}`, enabled, item_count })
export const KNOWN_FOLDERS: KnownFolderStatus[] = [
  kf('desktop', 'Desktop', 42), kf('documents', 'Documents', 1318), kf('pictures', 'Pictures', 2431),
  kf('music', 'Music', 86), kf('videos', 'Videos', 12), kf('downloads', 'Downloads', 377),
]

export const SAMPLE: PreviewSample = { label: '4K HEIC', size: '1.0 MB', original: '30.9 MB', photos: 2431, local: '2.4 GB', cloud: '2.4 GB', space: '20 GB' }
export const SUMMARY: SetupSummary = {
  passkey: 'Added', lock: 'On', backup: 'Desktop, Documents, Pictures', previews: '4K HEIC', explorer: 'On', notifications: 'On', login: 'On',
}
export const EXPLORER_ERROR = '0x8007017C'
