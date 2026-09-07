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
 * Module-level cache of the real host platform, filled in once by whichever
 * `usePlatform()` mount resolves first. `desktop_platform` never changes
 * within a window's lifetime, so every later mount (revisiting a settings
 * panel, mounting a sibling panel) reads this synchronously instead of
 * re-invoking the command and re-showing the unresolved fallback — that
 * repeated unresolved-then-resolved cycle is what produced the
 * Explorer -> Finder label flicker and the spurious "Windows shell
 * integration is only available on Windows" toast on macOS (PR #34 review).
 * `inFlight` dedupes concurrent callers during the single real IPC round trip.
 */
let cachedPlatform: PlatformName | null = null
let inFlight: Promise<void> | null = null

function resolvePlatformOnce(): Promise<void> {
  if (cachedPlatform !== null) return Promise.resolve()
  if (!inFlight) {
    inFlight = command<DesktopPlatform>('desktop_platform').then((result) => {
      if (result.ok && isPlatformName(result.value)) {
        cachedPlatform = result.value
      }
    })
  }
  return inFlight
}

export interface PlatformResolution {
  /** The real host platform, or `null` until `desktop_platform` resolves. */
  name: PlatformName | null
  /** True once `name` reflects the real host platform. */
  resolved: boolean
}

/**
 * React hook resolving the real host platform via the `desktop_platform`
 * Tauri command, with the resolution state exposed so callers can avoid
 * acting on the pre-resolution value (e.g. invoking an OS-specific command,
 * or rendering an OS-specific label) before it is safe to do so.
 */
export function usePlatform(): PlatformResolution {
  const [name, setName] = useState<PlatformName | null>(() => cachedPlatform)

  useEffect(() => {
    if (cachedPlatform !== null) {
      setName(cachedPlatform)
      return
    }
    let cancelled = false
    void resolvePlatformOnce().then(() => {
      if (!cancelled && cachedPlatform !== null) setName(cachedPlatform)
    })
    return () => {
      cancelled = true
    }
  }, [])

  return { name, resolved: name !== null }
}

/**
 * Convenience wrapper over `usePlatform()` for existing callers that just
 * want a best-effort platform name. Starts at `fallback` (defaulting to the
 * query-param hint, itself defaulting to 'windows' — this shell's historical
 * assumption) until the real platform resolves, then reflects the true OS.
 * Prefer `usePlatform()` directly for anything that must not act (call a
 * platform-specific command, render a platform-specific label) on the
 * unresolved fallback value.
 */
export function usePlatformName(fallback?: PlatformName): PlatformName {
  const { name } = usePlatform()
  return name ?? platformFromQueryParam(fallback ?? 'windows')
}
