/**
 * Home's counts line reads `desktop_file_overview` (task 1683 slice 4, the Home add-on): the state mirror's by-status
 * counts and the kept-folder count, computed locally. A hook, so `HomePane` stays one component; it asks while the vault
 * is unlocked (the command refuses a locked vault), on mount and every `FILE_OVERVIEW_REFRESH_MS`. A read that fails keeps
 * what the pane already shows, and a pane that has shown nothing shows nothing (no placeholder).
 */
import { useEffect, useState } from 'react'
import { desktopFileOverview, type FileOverview } from './desktopApi'

export const FILE_OVERVIEW_REFRESH_MS = 20_000

export function useFileOverview(enabled: boolean): FileOverview | null {
  const [overview, setOverview] = useState<FileOverview | null>(null)
  useEffect(() => {
    if (!enabled) {
      setOverview(null)
      return
    }
    let cancelled = false
    const read = async () => {
      // No recent files: the counts need the buckets and the totals, not the list.
      const result = await desktopFileOverview(0)
      if (!cancelled && result.ok) setOverview(result.value)
    }
    void read()
    const id = window.setInterval(() => void read(), FILE_OVERVIEW_REFRESH_MS)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [enabled])
  return overview
}
