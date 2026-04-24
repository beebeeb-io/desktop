// hifi-drive.jsx — Drive A hi-fi: sidebar + list

const HI_FILES = [
  { name: 'Contracts', type: 'folder', size: '—', mod: '2d ago', who: 'Anna K.', shared: 3 },
  { name: 'Q2 Financials', type: 'folder', size: '—', mod: '4h ago', who: 'Marc D.', shared: 2 },
  { name: 'Design Review', type: 'folder', size: '—', mod: '1w ago', who: 'Lena W.', shared: 5 },
  { name: 'board-deck-apr.pdf', type: 'pdf', size: '4.2 MB', mod: '12m ago', who: 'Anna K.', shared: 4 },
  { name: 'term-sheet-v3.docx', type: 'doc', size: '88 KB', mod: '1h ago', who: 'Marc D.', shared: 2 },
  { name: 'architecture.fig', type: 'fig', size: '12 MB', mod: 'yesterday', who: 'Lena W.', shared: 3 },
  { name: 'client-photos.zip', type: 'zip', size: '340 MB', mod: '3d ago', who: 'Pieter J.', shared: 1 },
  { name: 'notes.md', type: 'md', size: '6 KB', mod: 'just now', who: 'You', shared: 0 },
];

function FileIconHi({ type }) {
  const colors = {
    folder: 'var(--amber)',
    pdf: '#e85a4f',
    doc: '#3b82f6',
    fig: '#a855f7',
    zip: '#64748b',
    md: '#0f766e',
  };
  return (
    <div style={{
      width: 24, height: 24, borderRadius: 6,
      background: type === 'folder' ? 'var(--amber-bg)' : 'var(--paper-2)',
      border: '1px solid var(--line)',
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      color: colors[type] || 'var(--ink-3)',
      flexShrink: 0,
    }}>
      <Ico name={type === 'folder' ? 'folder' : 'file'} size={13} />
    </div>
  );
}

function AvatarStack({ n = 3 }) {
  const palette = ['#f5b800', '#e85a4f', '#3b82f6', '#a855f7', '#0f766e'];
  return (
    <div style={{ display: 'flex' }}>
      {Array.from({ length: Math.min(n, 3) }).map((_, i) => (
        <div key={i} style={{
          width: 18, height: 18, borderRadius: 999,
          background: palette[i], border: '1.5px solid var(--paper)',
          marginLeft: i === 0 ? 0 : -5, flexShrink: 0,
        }} />
      ))}
      {n > 3 && <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginLeft: 4, alignSelf: 'center' }}>+{n - 3}</span>}
    </div>
  );
}

function HiDrive() {
  return (
    <div className="bb-card elevated" style={{ width: 1040, height: 640, display: 'flex', overflow: 'hidden' }}>
      {/* Sidebar */}
      <div style={{
        width: 220, borderRight: '1px solid var(--line)',
        background: 'var(--paper-2)', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{ padding: '16px 16px 12px' }}>
          <BBLogo size={14} />
        </div>
        <div style={{ padding: '0 12px 10px' }}>
          <BBBtn variant="amber" style={{ width: '100%', justifyContent: 'center' }}>
            <Ico name="plus" size={13} /> New
          </BBBtn>
        </div>
        <div style={{ padding: '6px 12px' }}>
          {[
            ['folder', 'All files', true, null],
            ['users', 'Shared', false, '6'],
            ['image', 'Photos', false, '2.4k'],
            ['star', 'Starred', false, null],
            ['clock', 'Recent', false, null],
            ['trash', 'Trash', false, null],
          ].map(([ico, lbl, act, count]) => (
            <div key={lbl} className={'bb-side-item' + (act ? ' active' : '')}>
              <span className="bb-side-icon"><Ico name={ico} size={13} /></span>
              <span style={{ flex: 1 }}>{lbl}</span>
              {count && <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-4)' }}>{count}</span>}
            </div>
          ))}
        </div>

        <div className="bb-divider" style={{ margin: '10px 16px' }} />
        <div style={{ padding: '0 12px' }}>
          <div className="t-label" style={{ padding: '0 6px 8px' }}>Teams</div>
          {[['Acme Studio', true], ['Personal', false]].map(([t, amb]) => (
            <div key={t} className="bb-side-item">
              <span style={{
                width: 14, height: 14, borderRadius: 3,
                background: amb ? 'var(--amber)' : 'var(--paper-3)',
                border: '1px solid var(--line-2)',
              }} />
              <span style={{ flex: 1 }}>{t}</span>
            </div>
          ))}
        </div>

        {/* Storage */}
        <div style={{ marginTop: 'auto', padding: 16, borderTop: '1px solid var(--line)' }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Storage</div>
          <div className="bb-progress" style={{ marginBottom: 6 }}>
            <div style={{ width: '38%' }} />
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11 }}>
            <span className="t-mono num-tabular">76 / 200 GB</span>
            <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>Upgrade</span>
          </div>
          <div style={{ marginTop: 12 }}>
            <BBRegionBadge />
          </div>
        </div>
      </div>

      {/* Main */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
        {/* Topbar */}
        <div style={{ padding: '10px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 14 }}>
            <span style={{ color: 'var(--ink-3)' }}>All files</span>
            <Ico name="chevRight" size={12} color="var(--ink-4)" />
            <span style={{ color: 'var(--ink-3)' }}>Acme Studio</span>
            <Ico name="chevRight" size={12} color="var(--ink-4)" />
            <span style={{ fontWeight: 600 }}>Work</span>
          </div>
          <div className="bb-input" style={{ marginLeft: 'auto', width: 260, padding: '6px 10px' }}>
            <Ico name="search" size={13} color="var(--ink-4)" />
            <input placeholder="Search files and folders…" />
            <BBKbd>⌘K</BBKbd>
          </div>
          <BBBtn size="sm" icon={<Ico name="upload" size={13} />}>Upload</BBBtn>
          <BBBtn size="sm" variant="ghost" icon={<Ico name="more" size={14} />} />
        </div>

        {/* Column header */}
        <div style={{
          padding: '8px 20px', borderBottom: '1px solid var(--line)',
          display: 'grid', gridTemplateColumns: '32px 1fr 110px 110px 100px 60px', gap: 14,
          background: 'var(--paper-2)',
        }}>
          <span />
          <span className="t-label">Name</span>
          <span className="t-label">Size</span>
          <span className="t-label">Modified</span>
          <span className="t-label">Shared</span>
          <span />
        </div>

        {/* Rows */}
        <div className="bb-scroll" style={{ flex: 1 }}>
          {HI_FILES.map((f, i) => (
            <div key={i} className="bb-row" style={{
              gridTemplateColumns: '32px 1fr 110px 110px 100px 60px', gap: 14, padding: '11px 20px',
            }}>
              <FileIconHi type={f.type} />
              <div style={{ minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span style={{ fontWeight: 500 }}>{f.name}</span>
                  {i === 3 && <BBChip variant="amber" style={{ fontSize: 9.5 }}><Ico name="lock" size={9} /> E2EE</BBChip>}
                </div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{f.who}</div>
              </div>
              <span className="meta num-tabular">{f.size}</span>
              <span className="meta">{f.mod}</span>
              <div>{f.shared > 0 ? <AvatarStack n={f.shared} /> : <span className="meta">—</span>}</div>
              <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
                <BBBtn size="sm" variant="ghost" icon={<Ico name="more" size={14} />} />
              </div>
            </div>
          ))}
        </div>

        {/* Status bar */}
        <div style={{
          padding: '8px 20px', borderTop: '1px solid var(--line)',
          background: 'var(--paper-2)',
          display: 'flex', alignItems: 'center', gap: 14, fontSize: 11, color: 'var(--ink-3)',
        }}>
          <span className="t-mono">8 items</span>
          <span>·</span>
          <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <Ico name="shield" size={12} color="var(--amber-deep)" /> All encrypted · AES-256-GCM
          </span>
          <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 5 }}>
            <Ico name="cloud" size={12} /> Synced 14s ago
          </span>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiDrive, AvatarStack, FileIconHi });
