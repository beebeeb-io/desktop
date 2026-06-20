/**
 * Shared Windows app UI primitives (extracted from WindowsApp PKG-SHELL) —
 * consumed by WindowsApp + src/windows/views/*.
 */

// ── Design tokens (parallel to WindowsSettings.tsx) ─────────────────────────

export const T = {
  paper: 'oklch(0.985 0.004 85)',
  paper2: 'oklch(0.968 0.006 85)',
  paper3: 'oklch(0.945 0.008 82)',
  line: 'oklch(0.90 0.008 82)',
  line2: 'oklch(0.83 0.01 80)',
  ink: 'oklch(0.18 0.01 70)',
  ink2: 'oklch(0.34 0.012 75)',
  ink3: 'oklch(0.52 0.01 78)',
  ink4: 'oklch(0.68 0.008 80)',
  amber: 'oklch(0.82 0.17 84)',
  amberDeep: 'oklch(0.66 0.15 72)',
  amberBg: 'oklch(0.97 0.03 92)',
  green: 'oklch(0.72 0.16 155)',
  fontSans: "'Inter', 'Segoe UI', system-ui, ui-sans-serif, sans-serif",
  fontMono: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
} as const

// ── Mini icon set (superset of WindowsSettings' NavIcon) ─────────────────────

export function NavIcon({ name, size = 13, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  switch (name) {
    case 'home':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M2.5 7 L8 2.5 L13.5 7 L13.5 13 L9.5 13 L9.5 9.5 L6.5 9.5 L6.5 13 L2.5 13 Z" />
        </svg>
      )
    case 'user':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="5.5" r="2.5" />
          <path d="M2 14 C2 11 4.5 9 8 9 C11.5 9 14 11 14 14" strokeLinecap="round" />
        </svg>
      )
    case 'cloud':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M4.5 11 C2.5 11 2 8 4 8 C4 5.5 7 4 9.5 5 C11 4 14 5.5 13.5 8 C15.5 8 15.5 11 13.5 11 Z" />
        </svg>
      )
    case 'folder':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M2 5 L2 13 L14 13 L14 6 L7 6 L5.5 4 L2 4 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'bolt':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <path d="M9 2 L4 9 L8 9 L7 14 L12 7 L8 7 Z" />
        </svg>
      )
    case 'chart':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <path d="M2 13 L14 13" />
          <rect x="3.5" y="8" width="2.2" height="4" rx="0.5" fill={color} stroke="none" />
          <rect x="7" y="5" width="2.2" height="7" rx="0.5" fill={color} stroke="none" />
          <rect x="10.5" y="9.5" width="2.2" height="2.5" rx="0.5" fill={color} stroke="none" />
        </svg>
      )
    case 'device':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="2" y="3" width="9" height="7" rx="1.5" />
          <rect x="12" y="5" width="3" height="5" rx="1" />
          <line x1="2" y1="12" x2="11" y2="12" />
        </svg>
      )
    case 'shield':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M8 2 L13 4 L13 8 C13 11.5 10.5 13.5 8 14.5 C5.5 13.5 3 11.5 3 8 L3 4 Z" />
        </svg>
      )
    case 'clock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <circle cx="8" cy="8" r="6" />
          <path d="M8 5 L8 8 L10.5 9.5" />
        </svg>
      )
    case 'cog':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="8" r="2.5" />
          <path d="M8 1.5 L8 3.5 M8 12.5 L8 14.5 M1.5 8 L3.5 8 M12.5 8 L14.5 8 M3.5 3.5 L5 5 M11 11 L12.5 12.5 M12.5 3.5 L11 5 M5 11 L3.5 12.5" strokeLinecap="round" />
        </svg>
      )
    case 'lock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="3" y="7" width="10" height="7" rx="2" />
          <path d="M5 7 L5 5 C5 3.3 11 3.3 11 5 L11 7" />
        </svg>
      )
    case 'external':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M9 2 L14 2 L14 7 M14 2 L7.5 8.5" />
          <path d="M12 9.5 L12 13 L3 13 L3 4 L6.5 4" />
        </svg>
      )
    case 'check':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3.5 8.5 L6.5 11.5 L12.5 4.5" />
        </svg>
      )
    default:
      return <span style={{ display: 'block', width: s, height: s }} />
  }
}

// ── Shared primitives ───────────────────────────────────────────────────────

export function Chip({ children, tone = 'neutral' }: { children: React.ReactNode; tone?: 'neutral' | 'amber' | 'green' }) {
  const palette = {
    neutral: { bg: T.paper2, border: T.line2, color: T.ink3 },
    amber: { bg: T.amberBg, border: 'oklch(0.86 0.07 90)', color: 'oklch(0.4 0.08 72)' },
    green: { bg: 'oklch(0.96 0.04 155)', border: 'oklch(0.87 0.08 155)', color: 'oklch(0.4 0.1 155)' },
  }[tone]
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      padding: '1px 7px',
      fontSize: 10,
      fontFamily: T.fontMono,
      border: `1px solid ${palette.border}`,
      borderRadius: 999,
      background: palette.bg,
      color: palette.color,
      whiteSpace: 'nowrap' as const,
    }}>
      {children}
    </span>
  )
}

export function PrimaryBtn({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '8px 14px',
        fontSize: 12.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${T.ink}`,
        background: T.ink,
        color: T.paper,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.55 : 1,
        whiteSpace: 'nowrap' as const,
        letterSpacing: '-0.005em',
      }}
    >
      {children}
    </button>
  )
}

// A shimmering skeleton block — the canonical loading state for the data slots.
export function Skeleton({ width = '100%', height = 14, radius = 6 }: { width?: number | string; height?: number; radius?: number }) {
  return (
    <span
      aria-hidden
      style={{
        display: 'block',
        width,
        height,
        borderRadius: radius,
        background: `linear-gradient(90deg, ${T.paper3} 0%, ${T.paper2} 50%, ${T.paper3} 100%)`,
        backgroundSize: '200% 100%',
        animation: 'bb-shimmer 1.3s ease-in-out infinite',
      }}
    />
  )
}

// Reusable page header (title + subtitle).
export function PageHeader({ title, subtitle, aside }: { title: string; subtitle?: string; aside?: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 16, marginBottom: 20 }}>
      <div style={{ minWidth: 0 }}>
        <h1 style={{ margin: '0 0 6px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
          {title}
        </h1>
        {subtitle && <p style={{ margin: 0, fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 560 }}>{subtitle}</p>}
      </div>
      {aside}
    </div>
  )
}

export function Card({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ background: T.paper, border: `1px solid ${T.line}`, borderRadius: 10, boxShadow: 'var(--shadow-1)', ...style }}>
      {children}
    </div>
  )
}
