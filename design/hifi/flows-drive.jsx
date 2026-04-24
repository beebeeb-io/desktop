// flows-drive.jsx — File browser / drive view, 3 variants

const DRIVE_FILES = [
  { name: 'Contracts', type: 'folder', size: '—', mod: '2d ago' },
  { name: 'Q2 Financials', type: 'folder', size: '—', mod: '4h ago' },
  { name: 'Design Review', type: 'folder', size: '—', mod: '1w ago' },
  { name: 'board-deck-apr.pdf', type: 'pdf', size: '4.2 MB', mod: '12m ago' },
  { name: 'term-sheet-v3.docx', type: 'doc', size: '88 KB', mod: '1h ago' },
  { name: 'architecture.fig', type: 'fig', size: '12 MB', mod: 'yesterday' },
  { name: 'client-photos.zip', type: 'zip', size: '340 MB', mod: '3d ago' },
  { name: 'notes.md', type: 'md', size: '6 KB', mod: 'just now' },
];

function FileIcon({ type }) {
  const map = {
    folder: '▢', pdf: '▤', doc: '▦', fig: '◈', zip: '▩', md: '▥',
  };
  return <span style={{ fontFamily: 'var(--mono)', fontSize: 13, width: 14, display: 'inline-block' }}>{map[type] || '·'}</span>;
}

// ─── Variant A — Sidebar + list, conventional but clean ─────────────
function DriveA() {
  return (
    <div className="wf-card" style={{ width: 760, height: 480, fontFamily: 'var(--sans)', overflow: 'hidden', display: 'flex' }}>
      {/* Sidebar */}
      <div style={{ width: 180, borderRight: '1.2px solid var(--ink)', background: 'var(--paper-2)', padding: 14, display: 'flex', flexDirection: 'column', gap: 14 }}>
        <Logo size={14} />
        <Btn amber small style={{ justifyContent: 'center' }}>＋ New</Btn>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          {['Drive', 'Shared', 'Photos', 'Starred', 'Trash'].map((l, i) => (
            <div key={l} style={{
              padding: '6px 8px', fontSize: 12, borderRadius: 2,
              background: i === 0 ? 'var(--ink)' : 'transparent',
              color: i === 0 ? 'var(--paper)' : 'var(--ink)',
              fontWeight: i === 0 ? 600 : 400,
            }}>{l}</div>
          ))}
        </div>
        <Rule dashed />
        <div style={{ fontSize: 10, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.6 }}>Storage</div>
        <div>
          <div style={{ height: 5, border: '1px solid var(--ink)', marginBottom: 4 }}>
            <div style={{ width: '38%', height: '100%', background: 'var(--amber)' }} />
          </div>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-2)' }}>76 / 200 GB</div>
        </div>
        <div style={{ marginTop: 'auto' }}>
          <JurisdictionBadge region="FRA" small />
        </div>
      </div>

      {/* Main */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
        <div className="wf-row" style={{ padding: '10px 16px', borderBottom: '1.2px solid var(--ink)', gap: 10 }}>
          <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)' }}>Drive /</span>
          <span style={{ fontSize: 13, fontWeight: 600 }}>Work</span>
          <div style={{
            marginLeft: 'auto', flex: '0 1 260px', border: '1.1px solid var(--ink)',
            padding: '5px 9px', fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-4)',
          }}>⌕ search files…</div>
          <Btn alt small>⇅ Upload</Btn>
        </div>
        <div style={{ padding: '6px 16px', borderBottom: '1px dashed rgba(26,26,26,0.2)', display: 'grid', gridTemplateColumns: '18px 1fr 80px 90px 70px', gap: 10, fontFamily: 'var(--mono)', fontSize: 10, textTransform: 'uppercase', color: 'var(--ink-3)', letterSpacing: 0.6 }}>
          <span></span><span>Name</span><span>Size</span><span>Modified</span><span>Region</span>
        </div>
        <div className="wf-scroll" style={{ flex: 1 }}>
          {DRIVE_FILES.map((f, i) => (
            <div key={i} className="wf-file-row" style={{ gridTemplateColumns: '18px 1fr 80px 90px 70px' }}>
              <FileIcon type={f.type} />
              <span className="name">{f.name}</span>
              <span className="meta">{f.size}</span>
              <span className="meta">{f.mod}</span>
              <Chip style={{ fontSize: 9 }}>FRA</Chip>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Variant B — Honeycomb tile view ───────────────────────────────
function DriveB() {
  return (
    <div className="wf-card" style={{ width: 760, height: 480, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 18px', borderBottom: '1.2px solid var(--ink)' }}>
        <Logo size={14} />
        <div style={{ marginLeft: 20, fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-3)' }}>drive / work</div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <Chip>☰ List</Chip>
          <Chip filled>⬡ Hive</Chip>
          <Btn amber small>＋ Upload</Btn>
        </div>
      </div>

      <div style={{ padding: 20, display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: '10px 12px' }}>
        {DRIVE_FILES.slice(0, 10).concat(DRIVE_FILES.slice(0, 5)).map((f, i) => (
          <div key={i} style={{
            aspectRatio: '1 / 1.15',
            clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)',
            background: i === 0 ? 'var(--amber-soft)' : 'var(--paper-2)',
            border: '1.1px solid var(--ink)',
            display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
            gap: 4, padding: 6,
            marginTop: (i % 10 >= 5) ? 0 : 0,
            transform: (Math.floor(i / 5) % 2) ? 'translateX(36px)' : 'none',
          }}>
            <FileIcon type={f.type} />
            <div style={{ fontSize: 10, fontFamily: 'var(--mono)', textAlign: 'center', maxWidth: 80, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {f.name.length > 10 ? f.name.slice(0, 9) + '…' : f.name}
            </div>
            <div style={{ fontSize: 9, color: 'var(--ink-3)' }}>{f.size}</div>
          </div>
        ))}
      </div>

      <div style={{ position: 'absolute', right: 18, bottom: 14, fontFamily: 'var(--hand)', fontSize: 14, color: 'var(--ink-2)' }}>
        ↖ hex tile = one file. color = status.
      </div>
    </div>
  );
}

// ─── Variant C — Two-pane: list + preview with jurisdiction detail ───
function DriveC() {
  const sel = DRIVE_FILES[3];
  return (
    <div className="wf-card" style={{ width: 760, height: 480, fontFamily: 'var(--sans)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
      <div className="wf-row" style={{ padding: '10px 16px', borderBottom: '1.2px solid var(--ink)', gap: 10 }}>
        <Logo size={14} />
        <span style={{ marginLeft: 14, fontSize: 12, fontWeight: 600 }}>Work · Board</span>
        <Chip amber style={{ marginLeft: 6 }}>⬡ EU · FRA</Chip>
        <div style={{ marginLeft: 'auto', flex: '0 1 220px', border: '1.1px solid var(--ink)', padding: '5px 9px', fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-4)' }}>⌕ search…</div>
      </div>

      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <div className="wf-scroll" style={{ width: 340, borderRight: '1.2px solid var(--ink)' }}>
          {DRIVE_FILES.map((f, i) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '18px 1fr auto', gap: 10, alignItems: 'center',
              padding: '8px 14px', fontSize: 12,
              borderBottom: '1px dashed rgba(26,26,26,0.2)',
              background: i === 3 ? 'var(--amber-soft)' : 'transparent',
            }}>
              <FileIcon type={f.type} />
              <span>{f.name}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{f.mod}</span>
            </div>
          ))}
        </div>
        <div style={{ flex: 1, padding: 18, background: 'var(--paper-2)', display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div>
            <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.6 }}>Preview</div>
            <div style={{ fontSize: 14, fontWeight: 600, marginTop: 2 }}>{sel.name}</div>
          </div>
          <Placeholder h={130} label="PDF preview" />
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, fontSize: 11 }}>
            <div><div style={{ color: 'var(--ink-3)', fontFamily: 'var(--mono)', fontSize: 9, textTransform: 'uppercase' }}>Size</div>{sel.size}</div>
            <div><div style={{ color: 'var(--ink-3)', fontFamily: 'var(--mono)', fontSize: 9, textTransform: 'uppercase' }}>Encrypted</div>AES-256-GCM</div>
            <div><div style={{ color: 'var(--ink-3)', fontFamily: 'var(--mono)', fontSize: 9, textTransform: 'uppercase' }}>Region</div>Frankfurt · DE</div>
            <div><div style={{ color: 'var(--ink-3)', fontFamily: 'var(--mono)', fontSize: 9, textTransform: 'uppercase' }}>Shared with</div>2 people</div>
          </div>
          <Rule dashed />
          <div style={{ fontFamily: 'var(--hand)', fontSize: 13, color: 'var(--ink-2)' }}>
            Every detail shows where data lives. Trust, built in.
          </div>
          <div style={{ display: 'flex', gap: 6, marginTop: 'auto' }}>
            <Btn alt small>↗ Share</Btn>
            <Btn alt small>↓ Download</Btn>
            <Btn amber small style={{ marginLeft: 'auto' }}>Open</Btn>
          </div>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { DriveA, DriveB, DriveC });
