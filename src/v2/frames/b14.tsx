// Batch B14's frames (design/desktop-v2/BATCHES.md): account settings. Owned by B14 only.
import type { ReactElement } from 'react'
import { CaptionBar } from '../chrome/CaptionBar'
import { SettingsView } from '../sections/settings/shell/SettingsView'
import type { PaneId } from '../sections/settings/shell/context'
import { AccountPane, type AccountPage } from '../sections/settings/account/AccountPane'
import { DevicesPane } from '../sections/settings/account/DevicesPane'
import { SecurityPane, type SecurityInit } from '../sections/settings/account/SecurityPane'
import type { FrameRegistry } from './types'

function win(pane: PaneId, override: ReactElement, tall?: boolean) {
  return () => (
    <div className={`flex ${tall ? 'min-h-screen' : 'h-screen'} flex-col bg-paper font-sans text-ink`}>
      <CaptionBar title="Settings - Beebeeb" />
      <div className="flex min-h-0 flex-1 flex-col">
        <SettingsView pane={pane} tall={tall} override={override} />
      </div>
    </div>
  )
}

const account = (page: AccountPage, codeDigits?: string) => win('account', <AccountPane initial={page} codeDigits={codeDigits} />)
const security = (init: SecurityInit, tall?: boolean) => win('security', <SecurityPane init={init} />, tall)

export const FRAMES: FrameRegistry = {
  'DW-A51': account('main'),
  'DW-A52': security({}),
  'DW-A84': security({ page: 'checkup' }),
  'DW-A84a': security({ page: 'checkup', scenario: 'no-passkey' }),
  'DW-A55': win('devices', <DevicesPane />),
  'DW-A56': win('devices', <DevicesPane confirm="Anna’s iPhone" />),
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
  'DW-A77': security({ page: 'activity' }, true),
  'DW-A78': security({ page: 'activity', dialog: 'activity-off' }, true),
  'DW-A79': security({ page: 'activity', scenario: 'logging-off' }),
  'DW-A58': security({ page: 'delete-1' }, true),
  'DW-A80': security({ page: 'delete-2', digits: '71' }),
  'DW-A81': security({ page: 'delete-3' }),
  'DW-A82': security({ page: 'delete-4' }),
  'DW-A83': security({ page: 'delete-done' }),
}
