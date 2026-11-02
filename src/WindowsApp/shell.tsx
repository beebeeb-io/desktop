/**
 * Window shell pieces: the signed-out gate and the placeholder body a route without a view
 * renders. Both are deliberately small so the route switch reads as a list.
 */
import { useRegionLabel } from '../windows/useRegion'
import { T, NavIcon, Chip, PrimaryBtn, Skeleton, PageHeader, Card } from '../windows/ui'
import { NavId } from './nav'

// The honest placeholder body shown inside a not-yet-bound view. It clearly
// states the view is scaffolded and shows skeleton rows so the layout reads as
// "loading the shape" rather than "broken / empty".
export function PlaceholderBody({ note, dataSlot, rows = 3 }: { note: string; dataSlot: string; rows?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <Chip>Coming together</Chip>
          <span style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>{note}</span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {Array.from({ length: rows }).map((_, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '32px 1fr 90px', gap: 12, alignItems: 'center' }}>
              <Skeleton width={32} height={32} radius={8} />
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                <Skeleton width={`${60 - i * 8}%`} height={12} />
                <Skeleton width={`${40 - i * 5}%`} height={10} />
              </div>
              <Skeleton width={70} height={12} />
            </div>
          ))}
        </div>
      </Card>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, letterSpacing: '0.02em' }}>
        DATA SLOT — {dataSlot}
      </div>
    </div>
  )
}

// ── Signed-out gate ─────────────────────────────────────────────────────────

export function SignedOutGate({ onOpenSignIn }: { onOpenSignIn: () => void }) {
  const regionLabel = useRegionLabel()
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1, padding: '28px 36px', gap: 10 }}>
      <NavIcon name="lock" size={26} color={T.ink3} />
      <div style={{ fontSize: 16, fontWeight: 600, color: T.ink, marginTop: 6 }}>Sign in to Beebeeb</div>
      <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 320 }}>
        Your files are encrypted on this PC before they leave. {regionLabel}. Sign in to see your vault, devices, and account.
      </div>
      <div style={{ marginTop: 8 }}>
        <PrimaryBtn onClick={onOpenSignIn}>Sign in</PrimaryBtn>
      </div>
    </div>
  )
}

// Placeholder views — each declares the data wrapper(s) the next package binds.
export const PLACEHOLDER_VIEWS: Record<string, { title: string; subtitle: string; note: string; dataSlot: string }> = {
  account: {
    title: 'Account',
    subtitle: 'Your profile, plan, and subscription.',
    note: 'Email, plan, and subscription details land here.',
    dataSlot: 'accountProfile() · accountSubscription()',
  },
  insights: {
    title: 'Insights',
    subtitle: 'Where your storage goes — by content type and largest files.',
    note: 'Storage breakdown by Media / Documents / Other plus your largest files.',
    dataSlot: 'accountUsage() · accountStorageBreakdown()',
  },
  bandwidth: {
    title: 'Bandwidth',
    subtitle: 'Upload and download activity and per-transfer limits.',
    note: 'Live transfer rates and limits land here.',
    dataSlot: 'accountClientSessions() (speed_bps) · desktop_config',
  },
  'selective-sync': {
    title: 'Selective sync',
    subtitle: 'Pick which folders live on this PC.',
    note: 'The folder tree with online-only / pinned toggles lands here.',
    dataSlot: 'list_vault_folders · set_recursive_pin',
  },
  devices: {
    title: 'Devices',
    subtitle: 'Every device with access to your vault, and its sync sessions.',
    note: 'Registered devices and per-device sync sessions land here.',
    dataSlot: 'accountDevices() · accountClientSessions()',
  },
  security: {
    title: 'Security',
    subtitle: 'Your security score, sessions, and recovery posture.',
    note: 'Security score, active sessions, and factors land here.',
    dataSlot: 'accountSecurityScore() · accountSessionList()',
  },
  activity: {
    title: 'Activity',
    subtitle: 'A full audit log of what happened on your account.',
    note: 'The paginated audit-log feed lands here.',
    dataSlot: 'accountActivityFeed()',
  },
}

export function PlaceholderView({ navId }: { navId: NavId }) {
  const meta = PLACEHOLDER_VIEWS[navId]
  if (!meta) {
    return (
      <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
        <PageHeader title={navId} />
      </div>
    )
  }
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader title={meta.title} subtitle={meta.subtitle} />
      <PlaceholderBody note={meta.note} dataSlot={meta.dataSlot} rows={navId === 'activity' || navId === 'devices' ? 5 : 3} />
    </div>
  )
}
