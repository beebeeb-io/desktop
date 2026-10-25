/**
 * The constants the Mac Settings pane and the Windows views both show, defined once (PR #140 review, P3: they were
 * copied into `macSecurityModel`, `macSettingsModel`, `SecurityView`, `AccountView` and `SettingsView`, and a copy that
 * drifts is a link that opens the wrong page or a channel list that disagrees with itself). No imports: both sides load it.
 */

/** Where two-factor is set up and managed: the web app has the controls, this app has none (so none is promised). */
export const SECURITY_SETTINGS_URL = 'https://app.beebeeb.io/settings/security'

/** Where the password, the email and the billing are changed: the web app, in the browser (spec §7.8, "Account"). */
export const ACCOUNT_SETTINGS_URL = 'https://app.beebeeb.io/settings/account'

export type ReleaseChannelId = 'stable' | 'beta' | 'alpha'

/** The channels and what each is for: the Windows shell's words, kept (spec §10.2 #46: "Release channel"). */
export const RELEASE_CHANNELS: ReadonlyArray<{ value: ReleaseChannelId; label: string; hint: string }> = [
  { value: 'stable', label: 'Stable', hint: 'Regular releases after validation.' },
  { value: 'beta', label: 'Beta', hint: 'Earlier builds for release testing.' },
  { value: 'alpha', label: 'Alpha', hint: 'Earliest builds for rapid feedback.' },
]
