/**
 * Storage location: where the account keeps its encrypted data, and the strict-residency
 * switch. `DataResidencyContent` is exported because other surfaces render the same panel.
 */
import { T, Card, Chip, NavIcon, PageHeader, Skeleton, useToast } from '../../ui'
import { buildDataResidencyViewState, residencySummary, savedRegionMessage } from '../../dataResidencySettingsModel'
import { useDataResidency } from '../../useDataResidency'
import { RED, Toggle, SettingsSectionShell } from './primitives'

export function RegionSelectionCard({
  item,
  onSelect,
}: {
  item: ReturnType<typeof buildDataResidencyViewState>['items'][number]
  onSelect: (id: string) => void
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(item.id)}
      disabled={item.disabled}
      aria-pressed={item.selected}
      style={{
        width: '100%',
        textAlign: 'left',
        display: 'grid',
        gridTemplateColumns: '16px minmax(0, 1fr) 16px',
        gap: 12,
        alignItems: 'flex-start',
        padding: '14px 16px',
        borderRadius: 8,
        border: item.selected ? `1px solid ${T.amberDeep}` : `1px solid ${T.line}`,
        background: item.selected ? T.amberBg : item.disabled ? T.paper2 : T.paper,
        boxShadow: item.selected ? '0 0 0 1px color-mix(in oklch, var(--amber) 28%, transparent)' : 'none',
        opacity: item.disabled && !item.selected ? 0.58 : 1,
        cursor: item.disabled ? 'default' : 'pointer',
        transition: 'border-color 140ms ease, background 140ms ease, opacity 140ms ease',
      }}
    >
      <span
        aria-hidden
        style={{
          width: 16,
          height: 16,
          borderRadius: 999,
          border: item.selected ? `1.5px solid ${T.amberDeep}` : `1.5px solid ${T.line2}`,
          background: item.selected ? T.amber : 'transparent',
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          marginTop: 1,
        }}
      >
        {item.selected && <span style={{ width: 6, height: 6, borderRadius: 999, background: T.ink }} />}
      </span>

      <span style={{ minWidth: 0 }}>
        <span style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap' as const, gap: 7, marginBottom: 3 }}>
          <span style={{ fontSize: 13.5, fontWeight: 650, color: item.selected ? T.ink : T.ink2, lineHeight: 1.35 }}>
            {item.displayName}
          </span>
          {item.isDefault && <Chip>Default</Chip>}
          {item.comingSoon && <Chip>Coming soon</Chip>}
        </span>
        {item.detail && (
          <span style={{ display: 'block', fontFamily: T.fontMono, fontSize: 11.5, color: T.ink3, lineHeight: 1.45 }}>
            {item.detail}
          </span>
        )}
      </span>

      {item.selected && <NavIcon name="check" size={14} color={T.amberDeep} />}
    </button>
  )
}

export function DataResidencyPanel() {
  const { showToast } = useToast()
  const { loading, loadError, view, retry, select, setStrict } = useDataResidency()

  const handleStrict = async (next: boolean) => {
    const pinned = view.pinned
    const result = await setStrict(next)
    if (result.status === 'saved' && pinned) {
      showToast({
        variant: 'success',
        title: result.strict ? `Only storing in ${pinned.displayName}` : 'Other EU countries allowed',
        message: result.strict
          ? `If ${pinned.displayName} can’t take a new file, the upload stops.`
          : `If ${pinned.displayName} can’t take a new file, it goes to another EU country.`,
      })
      return
    }
    if (result.status === 'error') {
      showToast({
        variant: 'error',
        title: 'Failed to save storage location',
        message: result.message || 'Please try again.',
      })
    }
  }

  const handleSelect = async (id: string) => {
    const result = await select(id)
    if (result?.status === 'saved') {
      showToast({
        variant: 'success',
        title: 'Storage location updated',
        message: savedRegionMessage(view.items, result.preferredRegion),
      })
      return
    }

    if (result?.status === 'error') {
      showToast({
        variant: 'error',
        title: 'Failed to save storage location',
        message: result.message || 'Please try again.',
      })
    }
  }

  return <DataResidencyContent loading={loading} loadError={loadError} view={view} onRetry={retry} onSelect={handleSelect} onStrictChange={handleStrict} />
}

export function DataResidencyContent({ loading, loadError, view, onRetry, onSelect, onStrictChange }: {
  loading: boolean
  loadError: string | null
  view: ReturnType<typeof buildDataResidencyViewState>
  onRetry: () => void
  onSelect: (id: string) => void
  /** "Only store in <country>": offered only while a country is selected. */
  onStrictChange: (next: boolean) => void
}) {
  const summary = residencySummary(view)
  // A stored country that left the list is not a load failure: the rows are fine, none is selected.
  const error = loadError ?? (!summary && !view.preferenceUnavailable
    ? 'Your storage region could not be determined. Please try again.'
    : null)
  const subtitle = loading
    ? 'Loading data residency…'
    : error ? undefined
    : view.preferenceUnavailable
      ? 'Your chosen location isn’t available right now. New files go wherever Beebeeb places them by default.'
      : summary ?? undefined

  return (
    <SettingsSectionShell>
      <PageHeader
        title="Storage location"
        subtitle={subtitle}
      />

      {loading ? (
        <Card style={{ padding: 20, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <Skeleton width={170} height={13} />
          <Skeleton width="100%" height={48} radius={8} />
        </Card>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          {error && (
            <Card style={{ padding: 14, background: T.paper2, borderColor: 'oklch(0.88 0.05 25)' }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
                <NavIcon name="external" size={14} color={RED} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Could not load data residency</div>
                  <div style={{ fontSize: 11.5, lineHeight: 1.5, color: T.ink3, marginTop: 3, wordBreak: 'break-word' as const }}>
                    {error}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={onRetry}
                  style={{
                    border: `1px solid ${T.line2}`,
                    borderRadius: 6,
                    background: T.paper,
                    color: T.ink2,
                    padding: '6px 10px',
                    fontSize: 11.5,
                    fontWeight: 600,
                    cursor: 'pointer',
                  }}
                >
                  Retry
                </button>
              </div>
            </Card>
          )}

          {!error && <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {view.items.map(item => (
              <RegionSelectionCard
                key={item.id}
                item={item}
                onSelect={(id) => onSelect(id)}
              />
            ))}
          </div>}

          {!error && view.pinned && (
            <Card style={{ padding: 14 }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 14 }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontSize: 13, fontWeight: 650, color: T.ink2 }}>{view.pinned.strictLabel}</div>
                  <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55, marginTop: 3 }}>{view.pinned.strictHint}</div>
                </div>
                <Toggle
                  on={view.strict}
                  busy={view.saving}
                  onChange={onStrictChange}
                  label={view.pinned.strictLabel}
                />
              </div>
            </Card>
          )}

          {!error && !view.onlyOneRegion && <Card style={{ padding: 14, background: T.paper2 }}>
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
              <NavIcon name="shield" size={14} color={T.amberDeep} />
              <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55 }}>
                <strong style={{ color: T.ink2 }}>Existing files stay where they are.</strong>{' '}
                Only new uploads use the location you choose.
              </div>
            </div>
          </Card>}
        </div>
      )}
    </SettingsSectionShell>
  )
}
