// primitives.jsx — small shared bits for Beebeeb wireframes

function Hex({ size = 14, amber = true, style }) {
  return (
    <span
      className={amber ? 'wf-hexmark' : 'wf-hex-outline'}
      style={{ width: size, height: size * 1.15, display: 'inline-block', ...style }}
    />
  );
}

function Logo({ size = 18, word = true }) {
  return (
    <span className="wf-bee" style={{ fontSize: size, gap: 6 }}>
      <Hex size={size * 0.9} />
      {word && <span style={{ fontFamily: 'var(--sans)', fontWeight: 700 }}>beebeeb</span>}
    </span>
  );
}

function Chip({ children, amber, filled, style }) {
  return (
    <span
      className={'wf-chip' + (amber ? ' amber' : '') + (filled ? ' filled' : '')}
      style={style}
    >
      {children}
    </span>
  );
}

function Btn({ children, alt, amber, small, style }) {
  return (
    <span
      className={'wf-btn' + (alt ? ' alt' : '') + (amber ? ' amber' : '')}
      style={{ fontSize: small ? 10 : 12, padding: small ? '4px 8px' : '7px 12px', ...style }}
    >
      {children}
    </span>
  );
}

function Annot({ children, style }) {
  return <span className="wf-annot" style={style}>{children}</span>;
}

// Sketchy arrow (SVG) — use inside an absolutely-positioned wrapper.
function Arrow({ d, style, label }) {
  return (
    <svg style={{ position: 'absolute', overflow: 'visible', ...style }} width="100" height="60">
      <path d={d} className="wf-arrow" markerEnd="url(#ah)" />
      <defs>
        <marker id="ah" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto">
          <path d="M0,0 L10,5 L0,10" fill="none" stroke="#1a1a1a" strokeWidth="1.2" />
        </marker>
      </defs>
      {label && <text x="0" y="-4" fontFamily="Kalam, cursive" fontSize="12" fill="#3a3a3a">{label}</text>}
    </svg>
  );
}

// Sketchy placeholder rect (hatched)
function Placeholder({ w = '100%', h = 80, label, amber, style, children }) {
  return (
    <div
      className={amber ? 'wf-ph-amber' : 'wf-ph'}
      style={{
        width: w, height: h, display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)',
        textAlign: 'center', padding: 8, ...style,
      }}
    >
      {children || label}
    </div>
  );
}

// Dotted rule
function Rule({ dashed, style }) {
  return (
    <div
      style={{
        height: 0,
        borderTop: dashed ? '1px dashed rgba(26,26,26,0.3)' : '1px solid #1a1a1a',
        ...style,
      }}
    />
  );
}

// Small hand-written caption above/below a variant
function Caption({ children, style }) {
  return (
    <div
      style={{
        fontFamily: 'var(--hand)', fontSize: 15, color: 'var(--ink-2)',
        marginBottom: 6, ...style,
      }}
    >
      {children}
    </div>
  );
}

// Field (form input look)
function Field({ label, value, placeholder, hint, mono, style }) {
  return (
    <div style={{ ...style }}>
      {label && (
        <div style={{ fontSize: 10, fontFamily: 'var(--mono)', color: 'var(--ink-3)', marginBottom: 4, textTransform: 'uppercase', letterSpacing: 0.6 }}>
          {label}
        </div>
      )}
      <div style={{
        border: '1.1px solid var(--ink)', borderRadius: 3, padding: '7px 9px',
        background: 'var(--paper)', fontFamily: mono ? 'var(--mono)' : 'var(--sans)',
        fontSize: 12, color: value ? 'var(--ink)' : 'var(--ink-4)',
        minHeight: 16,
      }}>
        {value || placeholder || '\u00A0'}
      </div>
      {hint && (
        <div style={{ fontFamily: 'var(--hand)', fontSize: 12, color: 'var(--ink-3)', marginTop: 4 }}>
          {hint}
        </div>
      )}
    </div>
  );
}

// Jurisdiction badge (trust signal — surfaced everywhere)
function JurisdictionBadge({ region = 'FRA', small }) {
  return (
    <span className="wf-chip amber" style={{ fontSize: small ? 9 : 10 }}>
      <span style={{
        width: 6, height: 6, borderRadius: '50%', background: '#22c55e',
        display: 'inline-block', boxShadow: '0 0 0 1px #1a1a1a',
      }} />
      <span>EU · {region}</span>
    </span>
  );
}

// Tiny hex cluster motif (decorative corner)
function HexCluster({ style }) {
  return (
    <svg width="60" height="50" style={{ ...style }} viewBox="0 0 60 50">
      {[
        [15, 12], [35, 12], [25, 28], [15, 44], [35, 44],
      ].map(([cx, cy], i) => (
        <polygon
          key={i}
          points={`${cx},${cy-8} ${cx+7},${cy-4} ${cx+7},${cy+4} ${cx},${cy+8} ${cx-7},${cy+4} ${cx-7},${cy-4}`}
          fill={i === 2 ? '#f5b800' : 'none'}
          stroke="#1a1a1a"
          strokeWidth="1.1"
        />
      ))}
    </svg>
  );
}

Object.assign(window, { Hex, Logo, Chip, Btn, Annot, Arrow, Placeholder, Rule, Caption, Field, JurisdictionBadge, HexCluster });
