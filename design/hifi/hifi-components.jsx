// hifi-components.jsx — shared hi-fi primitives

function BBHex({ size = 14, style }) {
  return <span className="bb-hex" style={{ width: size, height: size * 1.15, display: 'inline-block', ...style }} />;
}

function BBLogo({ size = 15, word = true, style }) {
  return (
    <span className="bb-logo" style={{ fontSize: size, ...style }}>
      <BBHex size={size * 0.95} />
      {word && <span>beebeeb</span>}
    </span>
  );
}

function BBBtn({ children, variant = 'default', size, style, icon, onClick }) {
  const cls = 'bb-btn'
    + (variant === 'primary' ? ' primary' : '')
    + (variant === 'amber' ? ' amber' : '')
    + (variant === 'ghost' ? ' ghost' : '')
    + (variant === 'danger' ? ' danger' : '')
    + (size === 'sm' ? ' sm' : '')
    + (size === 'lg' ? ' lg' : '');
  return <span className={cls} style={style} onClick={onClick}>{icon && <span style={{ display: 'inline-flex' }}>{icon}</span>}{children}</span>;
}

function BBChip({ children, variant, style }) {
  const cls = 'bb-chip'
    + (variant === 'amber' ? ' amber' : '')
    + (variant === 'filled' ? ' filled' : '')
    + (variant === 'green' ? ' green' : '');
  return <span className={cls} style={style}>{children}</span>;
}

function BBRegionBadge({ region = 'Frankfurt, DE' }) {
  return (
    <span className="bb-chip green">
      <span className="dot" />
      <span style={{ color: 'oklch(0.35 0.05 160)', fontWeight: 500 }}>EU · {region}</span>
    </span>
  );
}

function BBCheck({ on }) {
  return <span className={'bb-check' + (on ? ' on' : '')} />;
}
function BBToggle({ on }) { return <span className={'bb-toggle' + (on ? ' on' : '')} />; }

function BBKbd({ children }) { return <span className="bb-kbd">{children}</span>; }

// Small icons drawn as clean SVG — line art, 1.5 stroke
function Ico({ name, size = 14, color = 'currentColor' }) {
  const props = { width: size, height: size, viewBox: '0 0 24 24', fill: 'none', stroke: color, strokeWidth: 1.6, strokeLinecap: 'round', strokeLinejoin: 'round' };
  const paths = {
    folder: <><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/></>,
    file: <><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></>,
    image: <><rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="m3 17 5-4 5 4 3-2 5 4"/></>,
    upload: <><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M17 8l-5-5-5 5"/><path d="M12 3v12"/></>,
    download: <><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></>,
    share: <><circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="M8.6 13.5l6.8 4"/><path d="M15.4 6.5l-6.8 4"/></>,
    lock: <><rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></>,
    shield: <><path d="M12 3l8 3v6c0 5-3.5 8.5-8 9-4.5-.5-8-4-8-9V6l8-3z"/></>,
    key: <><circle cx="8" cy="15" r="4"/><path d="m11 12 9-9"/><path d="m16 7 3 3"/></>,
    users: <><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></>,
    search: <><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></>,
    plus: <><path d="M12 5v14M5 12h14"/></>,
    star: <><path d="m12 3 2.9 6 6.6 1-4.8 4.6 1.1 6.6L12 18l-5.8 3.2 1.1-6.6L2.5 10l6.6-1L12 3z"/></>,
    trash: <><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h0a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v0a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></>,
    check: <><path d="M20 6 9 17l-5-5"/></>,
    chevRight: <><path d="m9 18 6-6-6-6"/></>,
    chevDown: <><path d="m6 9 6 6 6-6"/></>,
    copy: <><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></>,
    eye: <><path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7-11-7-11-7z"/><circle cx="12" cy="12" r="3"/></>,
    link: <><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7L12 5"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7L12 19"/></>,
    clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
    cloud: <><path d="M17.5 19a4.5 4.5 0 1 0-1.5-8.75A6 6 0 0 0 6 12a4 4 0 0 0 .5 8z"/></>,
    gallery: <><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></>,
    more: <><circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/></>,
    arrowUp: <><path d="M12 19V5"/><path d="m5 12 7-7 7 7"/></>,
    sparkle: <><path d="M12 3v4M12 17v4M3 12h4M17 12h4M5.6 5.6l2.8 2.8M15.6 15.6l2.8 2.8M5.6 18.4l2.8-2.8M15.6 8.4l2.8-2.8"/></>,
  };
  return <svg {...props} style={{ flexShrink: 0 }}>{paths[name]}</svg>;
}

Object.assign(window, { BBHex, BBLogo, BBBtn, BBChip, BBRegionBadge, BBCheck, BBToggle, BBKbd, Ico });
