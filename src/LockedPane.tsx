/**
 * Review and Trash while the vault is locked (task 1683 slice 4; spec §7.6: "Locked: one card, the popover's sentence
 * and 'Unlock vault'. Review and Trash show the same card."; hi-fi section 3, "Home, vault locked").
 *
 * The same words and the same button as Home's locked card, so the three places never disagree. Home keeps its own
 * inline copy of the card (`MacApp.tsx` `HomePane`), left alone here so this slice does not touch the code the Home
 * slice is changing.
 */
import { useState } from 'react'
import { command, commandUnavailableLabel } from './desktopApi'
import { Btn, SettingsIcon } from './macSettingsParts'
import { useToast } from './windows/ui'

export function LockedPane({ onRefresh }: { onRefresh: () => void }) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)

  const unlock = async () => {
    setBusy(true)
    const result = await command<void>('unlock_vault')
    setBusy(false)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t unlock the vault',
        message: result.unsupported ? commandUnavailableLabel('unlock_vault') : result.reason,
      })
    }
    onRefresh()
  }

  return (
    <div className="mw-content">
      <div className="mw-card mw-locked">
        <span className="mw-tile mw-tile--amber" aria-hidden="true">
          <SettingsIcon name="lock" size={16} />
        </span>
        <div className="mw-row-text">
          <div className="mw-row-title">Your vault is locked</div>
          <div className="mw-row-hint">Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.</div>
        </div>
        <Btn primary disabled={busy} onClick={() => void unlock()}>
          {busy ? 'Unlocking…' : 'Unlock vault'}
        </Btn>
      </div>
    </div>
  )
}
