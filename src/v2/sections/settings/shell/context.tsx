import { createContext, useContext } from 'react'

export type PaneId = 'general' | 'about' | 'account' | 'security' | 'devices' | 'notifications' | 'storage' | 'backups' | 'sync' | 'thumbnails' | 'compare' | 'backup-failures' | 'device'
export interface SettingsCtx {
  pane: PaneId
  go: (pane: PaneId) => void
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
