/**
 * The Data residency state shared by the Windows/Linux Settings panel and the macOS Settings
 * window (task 1919): load the storage-location list from the API, hold the stored preference,
 * and save a choice. One hook so the two windows cannot disagree about what a row means or what
 * is sent. The list is whatever `GET /me/region` returned; this file keeps no list of its own.
 */
import { useEffect, useState } from 'react'
import { accountRegion, commandUnavailableLabel, setAccountRegion, type StorageChoice } from '../desktopApi'
import {
  buildDataResidencyViewState,
  commitPreferredRegionSelection,
  loadDataResidency,
  type CommitPreferredRegionResult,
} from './dataResidencySettingsModel'

export function useDataResidency() {
  const [choices, setChoices] = useState<StorageChoice[]>([])
  const [preferredRegion, setPreferredRegion] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)

  const load = () => {
    let cancelled = false
    setLoading(true)
    setLoadError(null)
    void (async () => {
      const result = await loadDataResidency(accountRegion)
      if (cancelled) return
      setChoices(result.choices)
      setPreferredRegion(result.preferredRegion)
      setLoadError(result.error)
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => load(), [])

  const view = buildDataResidencyViewState({ preferredRegion, choices, saving })

  /** Send the row's id as it came from the API. A refusal rolls the row back and comes back as `error`. */
  const select = async (id: string): Promise<CommitPreferredRegionResult | null> => {
    const row = view.items.find(item => item.id === id)
    if (!row || row.disabled || saving || id === view.effectiveRegion) return null

    setSaving(true)
    const result = await commitPreferredRegionSelection({
      id,
      preferredRegion,
      saving,
      setPreferredRegion,
      savePreferredRegion: async next => {
        const response = await setAccountRegion(next)
        if (!response.ok) {
          throw new Error(response.unsupported ? commandUnavailableLabel('account_set_region') : response.reason)
        }
        return response.value
      },
    })
    setSaving(false)
    return result
  }

  return { loading, loadError, view, retry: load, select }
}
