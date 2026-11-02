/**
 * The sidebar storage widget: quota used, and the upgrade action when the plan is full. It reads
 * the numbers the window already loaded and never fetches.
 */
import { useRegionLabel } from '../windows/useRegion'
import { formatBytes, type BillingUsage, type StorageSummary } from '../desktopApi'
import { T, NavIcon, Skeleton } from '../windows/ui'

// ── Sidebar storage widget ──────────────────────────────────────────────────

export function StorageWidget({ usage, storage, onUpgrade }: { usage: BillingUsage | null; storage: StorageSummary | null; onUpgrade: () => void }) {
  const regionLabel = useRegionLabel()
  const usedBytes = usage?.used_bytes ?? storage?.used_bytes ?? null
  const quotaBytes = usage?.quota_bytes ?? storage?.quota_bytes ?? null
  const pct = usage?.percentage != null
    ? usage.percentage * 100
    : (usedBytes != null && quotaBytes != null && quotaBytes > 0 ? (usedBytes / quotaBytes) * 100 : null)

  return (
    <div style={{ marginTop: 'auto', padding: '14px 12px 10px', borderTop: `1px solid ${T.line}` }}>
      <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 8 }}>
        Storage
      </div>
      {pct != null ? (
        <div className="progress-track" style={{ marginBottom: 6 }}>
          <div className="progress-fill" style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
        </div>
      ) : (
        <div style={{ marginBottom: 6 }}><Skeleton height={8} radius={999} /></div>
      )}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: 11 }}>
        <span style={{ fontFamily: T.fontMono, color: T.ink3 }}>
          {usedBytes != null && quotaBytes != null ? `${formatBytes(usedBytes)} / ${formatBytes(quotaBytes)}` : '…'}
        </span>
        <button
          onClick={onUpgrade}
          style={{ background: 'transparent', border: 'none', padding: 0, cursor: 'pointer', color: T.amberDeep, fontWeight: 500, fontSize: 11, fontFamily: T.fontSans }}
        >
          Upgrade
        </button>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 12 }}>
        <NavIcon name="shield" size={11} color={T.amberDeep} />
        <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
          {regionLabel}
        </span>
      </div>
    </div>
  )
}
