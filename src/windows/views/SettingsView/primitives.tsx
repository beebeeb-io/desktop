/**
 * Settings primitives shared by more than one settings panel: the toggle, the error block,
 * the signed-out gate and the section shell. Each one is deliberately dumb — it renders what
 * it is given — so every panel keeps its own copy and behaviour decisions.
 */
import { type ReactNode } from 'react'
import { T, Card, NavIcon, PrimaryBtn } from '../../ui'

export const RED = 'var(--red)'

export function Toggle({ on, busy = false, onChange, label }: { on: boolean; busy?: boolean; onChange: (next: boolean) => void; label: string }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy}
      disabled={busy}
      onClick={() => onChange(!on)}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        border: 'none',
        padding: 2,
        background: on ? T.amberDeep : T.line2,
        cursor: busy ? 'progress' : 'pointer',
        opacity: busy ? 0.6 : 1,
        transition: 'background 150ms ease, opacity 150ms ease',
        display: 'inline-flex',
        alignItems: 'center',
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: 16,
          height: 16,
          borderRadius: '50%',
          background: T.paper,
          transform: on ? 'translateX(16px)' : 'translateX(0)',
          transition: 'transform 150ms ease',
          boxShadow: T.knobShadow,
        }}
      />
    </button>
  )
}

export function ErrorBlock({ reason, unsupported, onRetry }: { reason: string; unsupported: boolean; onRetry: () => void }) {
  if (unsupported) {
    return (
      <Card style={{ padding: 22, display: 'flex', alignItems: 'center', gap: 12 }}>
        <NavIcon name="clock" size={16} color={T.ink4} />
        <div>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink2 }}>Not available in this build</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5, marginTop: 2 }}>
            This setting binds to a command that is not wired into the current desktop build.
          </div>
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: 22 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <NavIcon name="external" size={16} color={RED} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Could not load</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5, marginTop: 4, wordBreak: 'break-word' as const }}>
            {reason}
          </div>
        </div>
      </div>
      <div style={{ marginTop: 14 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

export function SignedOutSettingsGate({ onOpenSignIn }: { onOpenSignIn: () => void }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', minHeight: 320, padding: '28px 36px', gap: 10 }}>
      <NavIcon name="lock" size={26} color={T.ink3} />
      <div style={{ fontSize: 16, fontWeight: 600, color: T.ink, marginTop: 6 }}>Sign in to manage this setting</div>
      <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 320 }}>
        Sync and notification preferences belong to your encrypted account.
      </div>
      <div style={{ marginTop: 8 }}>
        <PrimaryBtn onClick={onOpenSignIn}>Sign in</PrimaryBtn>
      </div>
    </div>
  )
}

export function SettingsSectionShell({ children }: { children: ReactNode }) {
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', minWidth: 0 }}>
      {children}
    </div>
  )
}
