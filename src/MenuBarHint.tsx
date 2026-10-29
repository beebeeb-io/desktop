/**
 * The Home window's one-time hint (task 1683 slice 6; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6).
 * Reads once whether it was dismissed; "Got it" hides it at once and saves that, so it never comes back on this Mac.
 * A save that fails changes nothing on screen: the hint is gone for the rest of this run (Home mounts this again on every
 * visit, so the run remembers it, `rememberHintDismissed`) and the next launch shows it once more.
 *
 * The component is a plain function declaration that reads only its props, hooks and the bindings imported above, so the
 * tests can execute it with the component harness.
 */
import { useEffect, useState } from 'react'
import { getMenuBarHintSeen, markMenuBarHintSeen } from './desktopApi'
import {
  hintDismissedThisRun,
  MENU_BAR_HINT_BODY,
  MENU_BAR_HINT_DISMISS,
  MENU_BAR_HINT_TITLE,
  menuBarHintState,
  rememberHintDismissed,
  type MenuBarHintState,
} from './menuBarHintModel'
import { Btn, SettingsIcon } from './macSettingsParts'

export function MenuBarHint() {
  const [state, setState] = useState<MenuBarHintState>('unknown')

  useEffect(() => {
    let cancelled = false
    void getMenuBarHintSeen().then((read) => {
      if (!cancelled) setState(menuBarHintState(read, hintDismissedThisRun()))
    })
    return () => {
      cancelled = true
    }
  }, [])

  if (state !== 'show') return null

  const dismiss = () => {
    rememberHintDismissed()
    setState('hide')
    void markMenuBarHintSeen()
  }

  return (
    <div className="mw-hint" data-home="hint" role="note" aria-label={MENU_BAR_HINT_TITLE}>
      <span className="mw-hint-icon" aria-hidden="true">
        <SettingsIcon name="info" size={18} />
      </span>
      <div className="mw-hint-text">
        <div className="mw-hint-title">{MENU_BAR_HINT_TITLE}</div>
        <div className="mw-hint-body">{MENU_BAR_HINT_BODY}</div>
      </div>
      <Btn onClick={dismiss}>{MENU_BAR_HINT_DISMISS}</Btn>
    </div>
  )
}
