// hifi-web-details.jsx — File details panel + context menu

// ─── File details right-rail panel ─────────────────────────────────────────
function HiFileDetails() {
  return (
    <div style={{
      width: 1040, height: 640, background: 'var(--paper)',
      borderRadius: 12, overflow: 'hidden', border: '1px solid var(--line-2)',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.22)',
      display: 'grid', gridTemplateColumns: '220px 1fr 320px',
    }}>
      {/* sidebar stub */}
      <div style={{ background: 'var(--paper-2)', borderRight: '1px solid var(--line)', padding: 16 }}>
        <BBLogo size={13} />
        <div style={{ marginTop: 20 }}>
          {['All files', 'Shared with me', 'Photos', 'Trash'].map((l, i) => (
            <div key={l} className={`bb-side-item ${i === 0 ? 'active' : ''}`}>
              <span className="bb-side-icon"><Ico name={['folder','share','image','trash'][i]} size={12} /></span>
              {l}
            </div>
          ))}
        </div>
      </div>

      {/* file list */}
      <div style={{ display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{
          padding: '14px 20px', borderBottom: '1px solid var(--line)',
          display: 'flex', alignItems: 'center', gap: 10,
        }}>
          <span className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>Vault / Stories / Port contract</span>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          {[
            { n: 'port-contract-draft-03.pdf', sz: '2.4 MB', mod: '2h ago', sel: true },
            { n: 'sources.md', sz: '12 KB', mod: 'yesterday' },
            { n: 'interview-03.m4a', sz: '48 MB', mod: '3d ago' },
            { n: 'leak-packet.zip', sz: '340 MB', mod: '1w ago' },
            { n: 'notes-march.md', sz: '6 KB', mod: '2w ago' },
          ].map((r, i) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '20px 1fr 90px 90px',
              alignItems: 'center', gap: 12, padding: '10px 20px',
              background: r.sel ? 'var(--amber-bg)' : 'transparent',
              borderLeft: r.sel ? '2px solid var(--amber-deep)' : '2px solid transparent',
              fontSize: 12.5,
            }}>
              <Ico name={r.n.endsWith('.pdf') ? 'file' : r.n.endsWith('.md') ? 'file' : 'file'} size={13} color={r.sel ? 'var(--amber-deep)' : 'var(--ink-3)'} />
              <span style={{ fontWeight: r.sel ? 500 : 400 }}>{r.n}</span>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r.sz}</span>
              <span className="t-micro" style={{ color: 'var(--ink-3)' }}>{r.mod}</span>
            </div>
          ))}
        </div>
      </div>

      {/* details panel */}
      <div style={{
        borderLeft: '1px solid var(--line)', background: 'var(--paper)',
        display: 'flex', flexDirection: 'column', overflow: 'auto',
      }}>
        <div style={{ padding: 20, borderBottom: '1px solid var(--line)' }}>
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
            <div style={{
              width: 44, height: 52, background: 'oklch(0.97 0.01 28)',
              border: '1px solid oklch(0.88 0.04 28)', borderRadius: 6,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              color: '#e85a4f', fontSize: 10, fontWeight: 700, letterSpacing: 0.5,
            }}>PDF</div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 600, lineHeight: 1.25, wordBreak: 'break-word' }}>port-contract-draft-03.pdf</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 3 }}>2.4 MB · 18 pages</div>
            </div>
            <Ico name="more" size={14} color="var(--ink-3)" />
          </div>
          <div style={{ display: 'flex', gap: 6, marginTop: 12 }}>
            <BBBtn size="sm" variant="amber" style={{ flex: 1, justifyContent: 'center' }}>
              <Ico name="share" size={11} color="var(--ink)" /> Share
            </BBBtn>
            <BBBtn size="sm" variant="ghost"><Ico name="download" size={11} /></BBBtn>
            <BBBtn size="sm" variant="ghost"><Ico name="star" size={11} /></BBBtn>
          </div>
        </div>

        <div style={{ padding: '18px 20px', borderBottom: '1px solid var(--line)' }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Who has access</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {[
              { n: 'Isa Marchetti (you)', r: 'Owner', i: 'IM' },
              { n: 'editor@example.eu', r: 'Can comment', i: 'EP' },
              { n: 'legal@example.eu', r: 'Can view', i: 'LP' },
            ].map((p, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  width: 24, height: 24, borderRadius: '50%',
                  background: ['oklch(0.7 0.1 55)', 'oklch(0.7 0.1 220)', 'oklch(0.7 0.1 155)'][i],
                  color: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 9.5, fontWeight: 600,
                }}>{p.i}</div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 11.5, fontWeight: 500 }}>{p.n}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{p.r}</div>
                </div>
              </div>
            ))}
          </div>
          <BBBtn size="sm" variant="ghost" style={{ marginTop: 10, width: '100%', justifyContent: 'center' }}>
            <Ico name="plus" size={11} /> Add people
          </BBBtn>
        </div>

        <div style={{ padding: '18px 20px', borderBottom: '1px solid var(--line)' }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Details</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
            {[
              ['Type', 'PDF document'],
              ['Location', 'Vault / Stories / Port contract'],
              ['Created', '12 Mar 2026'],
              ['Modified', '2 hours ago by you'],
              ['Size on disk', '2.4 MB (ciphertext 2.8 MB)'],
              ['Cipher', 'XChaCha20-Poly1305'],
              ['Region', 'Frankfurt, DE'],
            ].map(([k, v]) => (
              <div key={k} style={{ display: 'flex', fontSize: 11.5, gap: 12 }}>
                <span style={{ color: 'var(--ink-3)', width: 84, flexShrink: 0 }}>{k}</span>
                <span style={{ color: 'var(--ink-2)', flex: 1, wordBreak: 'break-word' }}>{v}</span>
              </div>
            ))}
          </div>
        </div>

        <div style={{ padding: '18px 20px' }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Recent activity</div>
          {[
            { t: 'You edited', s: '2 hours ago', ico: 'file' },
            { t: 'You shared with legal@', s: 'yesterday', ico: 'share' },
            { t: 'Anna K. viewed', s: '3 days ago', ico: 'eye' },
            { t: 'You uploaded', s: '5 days ago', ico: 'upload' },
          ].map((a, i, arr) => (
            <div key={i} style={{ display: 'flex', gap: 10, position: 'relative', paddingBottom: i < arr.length - 1 ? 14 : 0 }}>
              <div style={{
                width: 22, height: 22, borderRadius: '50%', flexShrink: 0,
                background: 'var(--paper-2)', border: '1px solid var(--line)',
                display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 1,
              }}>
                <Ico name={a.ico} size={10} color="var(--ink-3)" />
              </div>
              {i < arr.length - 1 && (
                <div style={{ position: 'absolute', left: 10.5, top: 22, bottom: -2, width: 1, background: 'var(--line)' }} />
              )}
              <div>
                <div style={{ fontSize: 11.5 }}>{a.t}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{a.s}</div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Right-click context menu ──────────────────────────────────────────────
function HiContextMenu() {
  const items = [
    { ico: 'eye', l: 'Open', kbd: '↵' },
    { ico: 'download', l: 'Download', kbd: '⌘S' },
    { ico: 'share', l: 'Share…', kbd: '⌘⇧S' },
    { div: true },
    { ico: 'copy', l: 'Duplicate', kbd: '⌘D' },
    { ico: 'folder', l: 'Move to…', kbd: '⌘⇧M' },
    { ico: 'star', l: 'Star', kbd: '⌘/' },
    { div: true },
    { ico: 'clock', l: 'Version history', sub: true },
    { ico: 'users', l: 'Manage access', sub: true },
    { ico: 'lock', l: 'Lock with extra passphrase…' },
    { div: true },
    { ico: 'trash', l: 'Move to trash', kbd: '⌫', danger: true },
  ];
  return (
    <div style={{
      width: 500, height: 420, background: 'oklch(0.97 0.005 80)',
      position: 'relative', overflow: 'hidden', borderRadius: 8,
      border: '1px solid var(--line)',
    }}>
      {/* faint file row behind */}
      <div style={{
        position: 'absolute', top: 40, left: 40, right: 40,
        padding: '10px 14px', background: 'var(--amber-bg)',
        border: '1px solid var(--amber-deep)', borderRadius: 6,
        display: 'flex', alignItems: 'center', gap: 10, fontSize: 12.5, opacity: 0.95,
      }}>
        <Ico name="file" size={13} color="var(--amber-deep)" />
        <span style={{ fontWeight: 500 }}>port-contract-draft-03.pdf</span>
        <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', marginLeft: 'auto' }}>2.4 MB</span>
      </div>

      {/* the menu */}
      <div style={{
        position: 'absolute', top: 70, left: 180,
        width: 260, background: 'var(--paper)',
        border: '1px solid var(--line-2)', borderRadius: 8,
        boxShadow: '0 18px 40px -12px rgba(0,0,0,0.22), 0 0 0 0.5px var(--line-2)',
        padding: 4, overflow: 'hidden',
      }}>
        {items.map((it, i) => it.div ? (
          <div key={i} style={{ height: 1, background: 'var(--line)', margin: '4px 2px' }} />
        ) : (
          <div key={i} style={{
            display: 'grid', gridTemplateColumns: '16px 1fr auto',
            alignItems: 'center', gap: 10, padding: '6px 10px',
            borderRadius: 5, fontSize: 12.5,
            background: i === 0 ? 'var(--ink)' : 'transparent',
            color: i === 0 ? 'var(--paper)' : it.danger ? 'oklch(0.5 0.16 28)' : 'var(--ink)',
          }}>
            <Ico name={it.ico} size={12} color={i === 0 ? 'var(--paper)' : it.danger ? 'oklch(0.5 0.16 28)' : 'var(--ink-3)'} />
            <span>{it.l}</span>
            {it.kbd && <span className="t-mono" style={{ fontSize: 10, opacity: 0.7 }}>{it.kbd}</span>}
            {it.sub && <Ico name="chevRight" size={10} color={i === 0 ? 'var(--paper)' : 'var(--ink-3)'} />}
          </div>
        ))}
      </div>
    </div>
  );
}

Object.assign(window, { HiFileDetails, HiContextMenu });
