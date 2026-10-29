import type { CSSProperties, ReactNode } from 'react'
import { Card, NavIcon, T } from './windows/ui'

/** Shared card treatment for capability explanations in either desktop shell. */
export function CapabilityNotice({ children, title, actions, role = 'status', style }: {
  children: ReactNode
  title?: string
  actions?: ReactNode
  role?: 'status' | 'alert'
  style?: CSSProperties
}) {
  return <Card style={{ padding: 18, ...style }}>
    <div className="capability-notice" role={role} style={{ display: 'flex', alignItems: 'flex-start', gap: 12, fontFamily: T.fontSans }}>
      <div aria-hidden="true" style={{ width: 32, height: 32, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        <NavIcon name="device" size={16} color={T.ink3} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        {title && <h2 style={{ margin: '0 0 6px', fontSize: 14, fontWeight: 600, lineHeight: 1.4, color: T.ink }}>{title}</h2>}
        <p style={{ margin: 0, fontSize: 12, lineHeight: 1.6, color: T.ink3 }}>{children}</p>
        {actions && <div className="button-row" style={{ marginTop: 14 }}>{actions}</div>}
      </div>
    </div>
  </Card>
}
