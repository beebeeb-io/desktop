import { Fragment, useEffect, useSyncExternalStore, type ReactNode } from 'react'
import { accountSessionRevision, subscribeAccountSession } from './accountSession'
import { observeSessionStatus } from './desktopApi'

/** Remount all account views, overlays and toast state together. Old async
 * callbacks target unmounted components; module caches are reset by the same
 * revision observation. Native polling also covers menu sign-out and other
 * WebViews, without depending on delivery of a transient event. Task 2243: the
 * poll reads only the session, from memory (no config file, no state DB), so it
 * can keep running in hidden windows every second. */
export default function AccountSessionBoundary({ children }: { children: ReactNode }) {
  const revision = useSyncExternalStore(subscribeAccountSession, accountSessionRevision)
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const refresh = async () => {
      await observeSessionStatus()
      if (!cancelled) timer = setTimeout(refresh, 1000)
    }
    void refresh()
    return () => { cancelled = true; clearTimeout(timer) }
  }, [])
  return revision === null ? null : <Fragment key={revision}>{children}</Fragment>
}
