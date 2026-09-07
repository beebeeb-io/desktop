/**
 * Real host-OS detection for platform-aware copy (strings that name an OS,
 * e.g. "sign in to Windows", "Explorer integration").
 *
 * This is deliberately NOT the same thing as the `?platform=windows` query
 * param `main.tsx` uses to pick which window shell to mount (`WindowsApp.tsx`
 * vs the compact `App.tsx`, see the comment block there). That param only
 * selects a component tree — it is not a promise that the underlying OS is
 * actually Windows. QA can (and does) force `platform=windows` on macOS
 * hardware to preview the Windows shell, so any component that shell renders
 * must ask the Rust side what the real host OS is (via the existing
 * `desktop_platform` command) rather than trusting the routing query param.
 */
import { useEffect, useState } from 'react'
import { command, type DesktopPlatform } from './desktopApi'

export type PlatformName = 'macos' | 'windows' | 'linux'

function isPlatformName(value: DesktopPlatform): value is PlatformName {
  return value === 'macos' || value === 'windows' || value === 'linux'
}

/** Best-effort synchronous hint from the window's own `?platform=` query param. */
function platformFromQueryParam(fallback: PlatformName): PlatformName {
  if (typeof window === 'undefined') return fallback
  const value = new URLSearchParams(window.location.search).get('platform')
  return value === 'macos' || value === 'windows' || value === 'linux' ? value : fallback
}

/**
 * React hook resolving the real host platform via the `desktop_platform`
 * Tauri command. Starts at `fallback` (defaulting to the query-param hint,
 * itself defaulting to 'windows' — this shell's historical assumption)
 * until the command resolves, then reflects the true OS.
 */
export function usePlatformName(fallback?: PlatformName): PlatformName {
  const [platform, setPlatform] = useState<PlatformName>(() => platformFromQueryParam(fallback ?? 'windows'))

  useEffect(() => {
    let cancelled = false
    command<DesktopPlatform>('desktop_platform').then((result) => {
      if (!cancelled && result.ok && isPlatformName(result.value)) {
        setPlatform(result.value)
      }
    })
    return () => {
      cancelled = true
    }
  }, [])

  return platform
}
