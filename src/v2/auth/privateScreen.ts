// B11 ruling / DW-0009: while a screen shows or takes a secret (the 12 words, the lock), Rust leaves the window out of
// screenshots, screen sharing and the Task View / Alt+Tab thumbnails. Outside Tauri this does nothing.
import { useEffect } from 'react'
import { command } from '../../desktopApi'
import { hasTauri } from '../source'

export function usePrivateScreen(): void {
  useEffect(() => {
    if (!hasTauri()) return
    void command('auth_screen_private', { on: true })
    return () => { void command('auth_screen_private', { on: false }) }
  }, [])
}
