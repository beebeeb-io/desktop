import { useEffect, useSyncExternalStore } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command } from './desktopApi'
import { useToast, type ToastInput } from './windows/ui'
import { connectNativeUpdateMenu, desktopUpdateCheck } from './windows/manualUpdateCheck'
import { buildUpdateCheckViewModel } from './windows/updateCheckViewModel'

/** Reuses the standard toast actions; available updates remain UpdateBanner's job. */
export function manualUpdateToast(state: ReturnType<typeof desktopUpdateCheck.getSnapshot>, retry: () => Promise<void>): ToastInput | null {
  if (state.kind === 'idle' || state.kind === 'update_available') return null
  const view = buildUpdateCheckViewModel(state, null, 'stable')
  return {
    id: 'desktop-update-check',
    variant: state.kind === 'error' ? 'error' : state.kind === 'up_to_date' ? 'success' : 'info',
    title: state.kind === 'checking' ? 'Checking for updates...' : view.title,
    message: state.kind === 'checking' ? 'Contacting the selected channel manifest.' : view.detail,
    action: state.kind === 'error' ? { label: 'Try again', onClick: retry } : undefined,
    durationMs: state.kind === 'error' || state.kind === 'checking' ? null : undefined,
  }
}

/** Mounted beside UpdateBanner in both app shells, independent of the active page. */
export default function ManualUpdateFeedback() {
  const state = useSyncExternalStore(desktopUpdateCheck.subscribe, desktopUpdateCheck.getSnapshot, desktopUpdateCheck.getSnapshot)
  const { showToast, dismissToast } = useToast()

  useEffect(() => connectNativeUpdateMenu(
    (callback) => listen('menu:check-for-updates', callback),
    () => command<boolean>('consume_menu_update_check'),
    desktopUpdateCheck.check,
  ), [])

  useEffect(() => {
    const toast = manualUpdateToast(state, desktopUpdateCheck.check)
    if (toast) showToast(toast)
    else dismissToast('desktop-update-check')
  }, [state, showToast, dismissToast])
  return null
}
