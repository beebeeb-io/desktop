// Account > Where your files are stored on macOS: the country rows the API returns and the
// "only store in <country>" switch. Keeps no country list of its own; the server's list wins.

import { Btn, Note, SettingRow, SettingsGroup, ToggleRow } from '../macSettingsParts'
import { residencySummary } from '../windows/dataResidencySettingsModel'
import { useDataResidency } from '../windows/useDataResidency'
import { useToast } from '../windows/ui'

// ── Account > Where your files are stored (task 1919) ──────────────────────

/**
 * The storage-location choice. The rows are the API's list (`GET /me/region` -> `choices`) through the
 * one hook the Windows panel uses: "Europe" is the default (any of our European data centers, the
 * server places the data), a live country is a row to pick, a coming-soon country is listed and cannot
 * be picked (task 2050: by country, never a city). This window keeps no list, city or country of its
 * own, so a country the server adds or retires changes these rows with no release. While a country is
 * chosen, "Only store in <country>" is a switch under the rows.
 *
 * One failure, one surface: a list that did not load is the inline block with Try again; a choice the
 * server refused is a toast (a one-off action that gates nothing) and the row goes back to what is saved.
 */
export function DataResidencyGroup() {
  const { showToast } = useToast()
  const { loading, loadError, view, retry, select, setStrict } = useDataResidency()

  const chooseStrict = async (next: boolean) => {
    const result = await setStrict(next)
    if (result.status !== 'error') return
    showToast({
      variant: 'error',
      title: 'Couldn’t change where files are stored',
      message: result.message || 'Please try again.',
    })
  }

  const choose = async (id: string) => {
    const result = await select(id)
    if (result?.status !== 'error') return
    showToast({
      variant: 'error',
      title: 'Couldn’t change where files are stored',
      message: result.message || 'Please try again.',
    })
  }

  if (loading) {
    return (
      <SettingsGroup title="Storage location">
        <SettingRow name="region-loading" label="Loading…" />
      </SettingsGroup>
    )
  }
  const summary = residencySummary(view)
  if (loadError !== null || (summary === null && !view.preferenceUnavailable)) {
    return (
      <SettingsGroup title="Storage location">
        <Note
          kind="alert"
          surface="data-residency-load"
          title="Couldn’t load storage locations"
          actions={<Btn onClick={() => void retry()}>Try again</Btn>}
        />
      </SettingsGroup>
    )
  }

  return (
    <SettingsGroup title="Storage location">
      <SettingRow
        name="region-summary"
        label={view.preferenceUnavailable ? 'Your chosen location isn’t available right now.' : summary}
        hint={view.preferenceUnavailable ? 'New files go wherever Beebeeb places them by default.' : undefined}
      />
      <div role="radiogroup" aria-label="Where your files are stored" className="ms-choices">
        {view.items.map((item) => (
          <button
            key={item.id}
            type="button"
            role="radio"
            className="ms-choice"
            data-row={`region-${item.id}`}
            aria-checked={item.selected}
            aria-labelledby={`ms-region-${item.id}-label`}
            aria-describedby={item.detail ? `ms-region-${item.id}-hint` : undefined}
            disabled={item.disabled}
            onClick={() => void choose(item.id)}
          >
            <span className="ms-radio" aria-hidden="true" />
            <span className="ms-row-text">
              <span className="ms-row-label" id={`ms-region-${item.id}-label`}>
                {item.displayName}
                {item.isDefault ? <span className="ms-tag">Default</span> : null}
                {item.comingSoon ? <span className="ms-tag">Coming soon</span> : null}
              </span>
              {item.detail ? (
                <span className="ms-row-hint" id={`ms-region-${item.id}-hint`}>
                  {item.detail}
                </span>
              ) : null}
            </span>
          </button>
        ))}
      </div>
      {view.pinned ? (
        <ToggleRow
          name="region-strict"
          label={view.pinned.strictLabel}
          hint={view.pinned.strictHint}
          on={view.strict}
          disabled={view.saving}
          onChange={(next) => void chooseStrict(next)}
        />
      ) : null}
      {view.onlyOneRegion ? null : (
        <Note kind="status">Existing files stay where they are. Only new files use the location you choose.</Note>
      )}
    </SettingsGroup>
  )
}
