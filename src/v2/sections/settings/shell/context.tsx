import { createContext, useContext } from 'react'

export type PaneId = 'general' | 'about' | 'account' | 'security' | 'devices' | 'notifications' | 'storage' | 'backups' | 'sync' | 'thumbnails' | 'compare' | 'backup-failures' | 'device' | 'bandwidth' | 'insights' | 'recent' | 'location' | 'diagnostics'
export const PANE_IDS: readonly PaneId[] = ['general', 'about', 'account', 'security', 'devices', 'notifications', 'storage', 'backups', 'sync', 'thumbnails', 'compare', 'backup-failures', 'device', 'bandwidth', 'insights', 'recent', 'location', 'diagnostics']

/** What a page opens with besides itself: a dialog over it, or the props of its first state (a sub-page of the pane). */
export interface GoOptions { dialog?: string; props?: Record<string, unknown> }

export interface SettingsCtx {
  pane: PaneId
  go: (pane: PaneId, opts?: GoOptions) => void
  dialog: string | null
  open: (dialog: string) => void
  close: () => void
}
export const Ctx = createContext<SettingsCtx | null>(null)
export function useSettings(): SettingsCtx {
  const c = useContext(Ctx)
  if (!c) throw new Error('useSettings outside SettingsView')
  return c
}
