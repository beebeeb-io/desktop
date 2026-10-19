/**
 * The Data residency state shared by the Windows/Linux Settings panel and the macOS Settings
 * window (task 1919): load the storage-location list from the API, hold the stored preference,
 * and save a choice. One hook so the two windows cannot disagree about what a row means or what
 * is sent. The list is whatever `GET /me/region` returned; this file keeps no list of its own.
 * Task 2050: the rows are countries; "Only store in <country>" (`region_strict`) rides on the same PUT.
 */
import { useEffect, useState } from 'react'
import { accountRegion, commandUnavailableLabel, setAccountRegion, type StorageChoice } from '../desktopApi'
import {
  buildDataResidencyViewState,
  commitPreferredRegionSelection,
  commitStrictSelection,
  loadDataResidency,
  type CommitPreferredRegionResult,
  type CommitStrictResult,
} from './dataResidencySettingsModel'

export function useDataResidency() {
  const [choices, setChoices] = useState<StorageChoice[]>([])
  const [preferredRegion, setPreferredRegion] = useState<string | null>(null)
  const [regionStrict, setRegionStrict] = useState(false)
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
      setRegionStrict(result.regionStrict)
      setLoadError(result.error)
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => load(), [])

  const view = buildDataResidencyViewState({ preferredRegion, choices, strict: regionStrict, saving })

  /** Send the row's id as it came from the API. A refusal rolls the row back and comes back as `error`. */
  const select = async (id: string): Promise<CommitPreferredRegionResult | null> => {
    const row = view.items.find(item => item.id === id)
    if (!row || row.disabled || saving || id === view.selectedId) return null

    // Strict is a promise about ONE country: it carries over to another country, and ends when the
    // person goes back to "Europe".
    const nextStrict = row.kind === 'country' && view.strict
    setSaving(true)
    const result = await commitPreferredRegionSelection({
      id,
      preferredRegion,
      saving,
      setPreferredRegion,
      savePreferredRegion: async next => {
        const response = await setAccountRegion(next, nextStrict)
        if (!response.ok) {
          throw new Error(response.unsupported ? commandUnavailableLabel('account_set_region') : response.reason)
        }
        setRegionStrict(response.value.region_strict ?? nextStrict)
        return response.value
      },
    })
    setSaving(false)
    return result
  }

  /** "Only store in <country>": re-sends the pinned country with the new value. */
  const setStrict = async (next: boolean): Promise<CommitStrictResult> => {
    setSaving(true)
    const result = await commitStrictSelection({
      pinnedId: view.pinned?.id ?? null,
      next,
      strict: view.strict,
      saving,
      setStrict: setRegionStrict,
      saveRegion: async (id, strict) => {
        const response = await setAccountRegion(id, strict)
        if (!response.ok) {
          throw new Error(response.unsupported ? commandUnavailableLabel('account_set_region') : response.reason)
        }
        return response.value
      },
    })
    setSaving(false)
    return result
  }

  return { loading, loadError, view, retry: load, select, setStrict }
}
