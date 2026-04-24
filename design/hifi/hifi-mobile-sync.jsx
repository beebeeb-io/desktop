// hifi-mobile-sync.jsx — iOS A, Android B, Sync A/B hi-fi

function PhoneShell({ children, platform = 'ios', width = 320, height = 660 }) {
  return (
    <div style={{
      width, height, background: '#0b0b0c', borderRadius: 44, padding: 8,
      boxShadow: '0 30px 60px -20px rgba(0,0,0,0.35), 0 0 0 1.5px #1a1a1d, inset 0 0 0 1px #2a2a2d',
      position: 'relative',
    }}>
      <div style={{
        width: '100%', height: '100%', borderRadius: 36, overflow: 'hidden',
        background: 'var(--paper)', position: 'relative',
      }}>
        {platform === 'ios' && (
          <>
            <div style={{ position: 'absolute', top: 10, left: '50%', transform: 'translateX(-50%)', width: 96, height: 28, background: '#0b0b0c', borderRadius: 20, zIndex: 10 }} />
            <div style={{ position: 'absolute', top: 15, left: 24, right: 24, display: 'flex', justifyContent: 'space-between', fontSize: 12, fontWeight: 600, zIndex: 11 }}>
              <span>9:41</span>
              <span style={{ fontSize: 10, opacity: 0.7, marginRight: 110 }}></span>
              <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                <span style={{ width: 14, height: 8, borderRadius: 2, border: '1px solid var(--ink)', position: 'relative' }}>
                  <span style={{ position: 'absolute', inset: 1, width: '75%', background: 'var(--ink)', borderRadius: 1 }} />
                </span>
              </span>
            </div>
          </>
        )}
        {platform === 'android' && (
          <div style={{ position: 'absolute', top: 8, left: 20, right: 20, display: 'flex', justifyContent: 'space-between', fontSize: 11, fontWeight: 600, zIndex: 11 }}>
            <span>9:41</span>
            <span>●●● 81%</span>
          </div>
        )}
        <div style={{ position: 'absolute', inset: 0, paddingTop: platform === 'ios' ? 52 : 28 }}>
          {children}
        </div>
      </div>
    </div>
  );
}

function HiIOS() {
  const files = [
    { n: 'Contracts', t: 'folder', c: 12, d: '2d' },
    { n: 'Q2 Financials', t: 'folder', c: 8, d: '4h' },
    { n: 'board-deck.pdf', t: 'pdf', c: '4.2 MB', d: '12m' },
    { n: 'term-sheet.docx', t: 'doc', c: '88 KB', d: '1h' },
    { n: 'design.fig', t: 'fig', c: '12 MB', d: 'yest.' },
    { n: 'photos.zip', t: 'zip', c: '340 MB', d: '3d' },
  ];
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '6px 18px 8px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
            <BBLogo size={14} />
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
              <div style={{ width: 28, height: 28, borderRadius: 999, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="search" size={13} color="var(--ink-2)" />
              </div>
              <div style={{ width: 28, height: 28, borderRadius: 999, background: '#f5b800', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="plus" size={14} />
              </div>
            </div>
          </div>
          <div className="t-display" style={{ fontSize: 28, marginBottom: 2 }}>Drive</div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <BBRegionBadge region="Frankfurt" />
          </div>
        </div>
        <div style={{ flex: 1, overflow: 'hidden', padding: '4px 0' }}>
          {files.map((f, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', gap: 12,
              padding: '10px 18px',
              borderTop: i === 0 ? '1px solid var(--line)' : 'none',
              borderBottom: '1px solid var(--line)',
            }}>
              <FileIconHi type={f.t} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 14, fontWeight: 500 }}>{f.n}</div>
                <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>
                  {typeof f.c === 'number' ? `${f.c} items` : f.c} · {f.d}
                </div>
              </div>
              <Ico name="chevRight" size={13} color="var(--ink-4)" />
            </div>
          ))}
        </div>
        <div style={{ display: 'flex', padding: '8px 24px 20px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)' }}>
          {[['folder', 'Files', true], ['users', 'Shared', false], ['image', 'Photos', false], ['settings', 'Settings', false]].map(([ico, l, a], i) => (
            <div key={i} style={{ flex: 1, textAlign: 'center', color: a ? 'var(--ink)' : 'var(--ink-4)' }}>
              <Ico name={ico} size={18} />
              <div style={{ fontSize: 9.5, marginTop: 2, fontWeight: a ? 600 : 400 }}>{l}</div>
            </div>
          ))}
        </div>
      </div>
    </PhoneShell>
  );
}

function HiAndroid() {
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column', padding: '14px 16px 0' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
          <div style={{ width: 32, height: 32, borderRadius: 999, background: 'var(--amber)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <span style={{ fontWeight: 700, fontSize: 12 }}>A</span>
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>Good evening,</div>
            <div style={{ fontSize: 15, fontWeight: 600 }}>Anna</div>
          </div>
          <Ico name="settings" size={16} color="var(--ink-3)" />
        </div>

        <div className="bb-input" style={{ marginBottom: 14, padding: '10px 12px', borderRadius: 14 }}>
          <Ico name="search" size={14} color="var(--ink-4)" />
          <input placeholder="Search everything…" style={{ fontSize: 13 }} />
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, marginBottom: 14 }}>
          {[
            { ico: 'upload', l: 'Upload', bg: 'var(--ink)', c: 'var(--paper)' },
            { ico: 'image', l: 'Scan', bg: 'var(--amber)', c: 'var(--ink)' },
            { ico: 'share', l: 'Send', bg: 'var(--paper-2)', c: 'var(--ink)' },
            { ico: 'lock', l: 'Vault', bg: 'var(--paper-2)', c: 'var(--ink)' },
          ].map((a, i) => (
            <div key={i} style={{
              padding: '14px 12px', borderRadius: 14, background: a.bg, color: a.c,
              border: a.bg === 'var(--paper-2)' ? '1px solid var(--line)' : 'none',
              display: 'flex', flexDirection: 'column', gap: 8, minHeight: 72,
            }}>
              <Ico name={a.ico} size={18} />
              <div style={{ fontSize: 13, fontWeight: 600 }}>{a.l}</div>
            </div>
          ))}
        </div>

        <div className="t-label" style={{ marginBottom: 8 }}>Recent</div>
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {[
            { n: 'board-deck.pdf', t: 'pdf', d: '12m' },
            { n: 'term-sheet.docx', t: 'doc', d: '1h' },
            { n: 'design.fig', t: 'fig', d: 'yesterday' },
          ].map((f, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', gap: 10,
              padding: '10px 2px', borderBottom: '1px solid var(--line)',
            }}>
              <FileIconHi type={f.t} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 500 }}>{f.n}</div>
                <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>{f.d}</div>
              </div>
              <Ico name="more" size={14} color="var(--ink-4)" />
            </div>
          ))}
        </div>

        <div style={{ display: 'flex', padding: '10px 0 14px' }}>
          {[['folder', 'Files', true], ['star', 'Favs', false], ['users', 'Shared', false], ['image', 'Me', false]].map(([ico, l, a], i) => (
            <div key={i} style={{ flex: 1, textAlign: 'center', color: a ? 'var(--amber-deep)' : 'var(--ink-4)' }}>
              <Ico name={ico} size={18} />
              <div style={{ fontSize: 9.5, marginTop: 2, fontWeight: a ? 600 : 400 }}>{l}</div>
            </div>
          ))}
        </div>
      </div>
    </PhoneShell>
  );
}

function SyncPopover({ state = 'active' }) {
  const active = state === 'active';
  const items = active ? [
    { n: 'term-sheet-v3.docx', p: 62, stage: 'Encrypting' },
    { n: 'client-photos.zip', p: 18, stage: 'Uploading' },
  ] : [];
  return (
    <div className="bb-card elevated" style={{ width: 340, fontFamily: 'var(--font-sans)', borderRadius: 12 }}>
      {/* Title area */}
      <div style={{ padding: '12px 14px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{
          width: 26, height: 26, borderRadius: 999,
          background: active ? 'var(--amber-bg)' : 'oklch(0.96 0.04 155)',
          border: '1px solid',
          borderColor: active ? 'oklch(0.86 0.07 90)' : 'oklch(0.88 0.07 155)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          color: active ? 'var(--amber-deep)' : 'oklch(0.45 0.12 155)',
        }}>
          <Ico name={active ? 'cloud' : 'check'} size={13} />
        </div>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>
            {active ? 'Syncing 2 files' : 'All files up to date'}
          </div>
          <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>
            {active ? '218 MB / 352 MB · 14 MB/s' : 'Last sync 14s ago'}
          </div>
        </div>
        <BBBtn size="sm" variant="ghost" icon={<Ico name="settings" size={13} />} />
      </div>

      {active ? (
        <div style={{ padding: '10px 14px 12px' }}>
          {items.map((f, i) => (
            <div key={i} style={{ padding: '6px 0', borderBottom: i < items.length - 1 ? '1px solid var(--line)' : 'none' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Ico name="file" size={12} color="var(--ink-3)" />
                <span style={{ fontSize: 12.5, fontWeight: 500, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.n}</span>
                <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--amber-deep)', fontWeight: 500 }}>{f.stage} {f.p}%</span>
              </div>
              <div className="bb-progress" style={{ marginTop: 5, height: 3 }}>
                <div style={{ width: `${f.p}%` }} />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div style={{ padding: '10px 14px' }}>
          <div className="t-label" style={{ marginBottom: 6 }}>Recently synced</div>
          {['term-sheet-v3.docx', 'board-deck-apr.pdf', 'notes.md'].map((n, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 0', fontSize: 12 }}>
              <Ico name="check" size={11} color="oklch(0.45 0.12 155)" />
              <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{n}</span>
              <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-4)' }}>{['14s', '4m', '1h'][i]}</span>
            </div>
          ))}
        </div>
      )}

      <div className="bb-divider" />

      <div style={{ padding: '10px 14px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
        {[['folder', 'Open folder'], ['cloud', 'beebeeb.io'], ['clock', 'Activity'], ['lock', 'Vault']].map(([ico, l], i) => (
          <div key={i} style={{
            display: 'flex', alignItems: 'center', gap: 7,
            padding: '6px 8px', borderRadius: 5, fontSize: 12, color: 'var(--ink-2)',
            cursor: 'default',
          }}>
            <Ico name={ico} size={12} color="var(--ink-3)" /> {l}
          </div>
        ))}
      </div>

      <div style={{
        padding: '9px 14px', borderTop: '1px solid var(--line)',
        background: 'var(--paper-2)',
        display: 'flex', alignItems: 'center', gap: 8, fontSize: 11, color: 'var(--ink-3)',
      }}>
        <BBRegionBadge region="Frankfurt" />
        <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 4, color: 'var(--ink-3)' }}>
          <Ico name="shield" size={10} color="var(--amber-deep)" /> E2EE
        </span>
      </div>
    </div>
  );
}

function HiSyncActive() {
  return (
    <div style={{ position: 'relative', width: 420, height: 320, background: 'linear-gradient(180deg, oklch(0.92 0.02 85), oklch(0.84 0.03 80))', borderRadius: 10, overflow: 'hidden' }}>
      <div style={{ position: 'absolute', top: 8, left: 10, display: 'flex', alignItems: 'center', gap: 14, fontSize: 11, color: 'rgba(0,0,0,0.7)', fontWeight: 500 }}>
        <BBHex size={10} />
        <span></span>
        <span>File</span>
        <span>Edit</span>
        <span>View</span>
      </div>
      <div style={{ position: 'absolute', top: 40, right: 20 }}>
        <SyncPopover state="active" />
      </div>
      <div style={{ position: 'absolute', top: 8, right: 16, display: 'flex', alignItems: 'center', gap: 10, fontSize: 11, color: 'rgba(0,0,0,0.7)' }}>
        <BBHex size={9} />
        <span className="t-mono">81%</span>
        <span>9:41</span>
      </div>
    </div>
  );
}

function HiSyncIdle() {
  return (
    <div style={{ position: 'relative', width: 420, height: 320, background: 'linear-gradient(180deg, oklch(0.92 0.02 85), oklch(0.84 0.03 80))', borderRadius: 10, overflow: 'hidden' }}>
      <div style={{ position: 'absolute', top: 8, left: 10, display: 'flex', alignItems: 'center', gap: 14, fontSize: 11, color: 'rgba(0,0,0,0.7)', fontWeight: 500 }}>
        <BBHex size={10} />
        <span></span>
        <span>File</span>
        <span>Edit</span>
        <span>View</span>
      </div>
      <div style={{ position: 'absolute', top: 40, right: 20 }}>
        <SyncPopover state="idle" />
      </div>
      <div style={{ position: 'absolute', top: 8, right: 16, display: 'flex', alignItems: 'center', gap: 10, fontSize: 11, color: 'rgba(0,0,0,0.7)' }}>
        <BBHex size={9} />
        <span className="t-mono">82%</span>
        <span>9:41</span>
      </div>
    </div>
  );
}

Object.assign(window, { HiIOS, HiAndroid, HiSyncActive, HiSyncIdle, PhoneShell });
