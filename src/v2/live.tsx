// Batch M2: the live state route's door. On the dev server in a plain browser, a v2 window first loads ./dev/live, which
// answers the read commands from the one fixture set and, for `?state=DW-xxxx`, drives the REAL window into that board
// frame's state (see ./dev/live.ts). In a production build `import.meta.env.DEV` is false: the window is returned as
// it is and ./dev is never bundled. Inside Tauri the real backend answers, so nothing is installed there either.
import { useEffect, useState, type ComponentType } from 'react'
import { hasTauri } from './source'

export type LiveWindow = 'main-app' | 'tray' | 'onboarding'

export function withLiveState<P extends object>(Window: ComponentType<P>, kind: LiveWindow): ComponentType<P> {
  if (!import.meta.env.DEV) return Window
  function LiveStateWindow(props: P) {
    const [ready, setReady] = useState(() => hasTauri())
    useEffect(() => {
      if (ready) return
      let live = true
      void import('./dev/live').then((m) => m.prepare(kind)).then(() => { if (live) setReady(true) })
      return () => { live = false }
    }, [ready])
    return ready ? <Window {...props} /> : null
  }
  return LiveStateWindow
}
