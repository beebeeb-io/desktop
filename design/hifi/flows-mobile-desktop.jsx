// flows-mobile-desktop.jsx — iOS, Android, Desktop sync client

// ─── iOS Drive ─────────────────────────────────────────────────────────
function IOSDrive({ variant = 'A' }) {
  const files = [
    ['Contracts', 'folder', '12 items'],
    ['Q2 Financials', 'folder', '4 items'],
    ['board-deck.pdf', 'pdf', '4.2 MB'],
    ['term-sheet.docx', 'doc', '88 KB'],
    ['notes.md', 'md', '6 KB'],
  ];
  return (
    <div style={{ background: 'var(--paper)', width: '100%', height: '100%', fontFamily: 'var(--sans)', color: 'var(--ink)', display: 'flex', flexDirection: 'column' }}>
      {/* top */}
      <div style={{ padding: '14px 18px 8px' }}>
        <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 10 }}>
          <JurisdictionBadge region="FRA" small />
          <span style={{ fontSize: 16 }}>⬡</span>
        </div>
        <div style={{ fontFamily: 'var(--hand)', fontSize: 30, lineHeight: 1, marginBottom: 4 }}>
          {variant === 'C' ? 'Hive' : 'Drive'}
        </div>
        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)' }}>76 / 200 GB · encrypted</div>
      </div>

      {variant === 'A' && (
        <div style={{ padding: '0 14px', flex: 1, overflow: 'hidden' }}>
          <div style={{ border: '1.1px solid var(--ink)', padding: '6px 10px', fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-4)', marginBottom: 10 }}>⌕ search</div>
          {files.map((f, i) => (
            <div key={i} className="wf-row" style={{ padding: '10px 4px', borderBottom: '1px dashed rgba(26,26,26,0.2)', gap: 10 }}>
              <span style={{ fontFamily: 'var(--mono)' }}>{f[1] === 'folder' ? '▢' : '▤'}</span>
              <span style={{ fontSize: 13, flex: 1 }}>{f[0]}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{f[2]}</span>
            </div>
          ))}
        </div>
      )}

      {variant === 'B' && (
        <div style={{ padding: 14, flex: 1 }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
            {files.slice(0, 4).map((f, i) => (
              <div key={i} className="wf-card" style={{ padding: 12, boxShadow: '2px 2px 0 rgba(26,26,26,0.85)' }}>
                <div style={{ fontSize: 18, marginBottom: 12 }}>{f[1] === 'folder' ? '▢' : '▤'}</div>
                <div style={{ fontSize: 12, fontWeight: 600 }}>{f[0]}</div>
                <div style={{ fontSize: 10, fontFamily: 'var(--mono)', color: 'var(--ink-3)' }}>{f[2]}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {variant === 'C' && (
        <div style={{ padding: 14, flex: 1, display: 'flex', flexDirection: 'column', gap: 8 }}>
          {[0, 1, 2].map((row) => (
            <div key={row} style={{ display: 'flex', gap: 6, justifyContent: 'center', marginLeft: row % 2 ? 26 : 0, marginTop: row === 0 ? 0 : -6 }}>
              {Array.from({ length: 3 }).map((_, i) => (
                <div key={i} style={{
                  width: 72, height: 82,
                  clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)',
                  background: (row + i) === 1 ? 'var(--amber-soft)' : 'var(--paper-2)',
                  border: '1px solid var(--ink)',
                  display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                  fontSize: 9, fontFamily: 'var(--mono)',
                }}>
                  <span style={{ fontSize: 14 }}>⬡</span>
                  <span>{files[(row * 3 + i) % files.length][0].slice(0, 8)}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
      )}

      {/* tab bar */}
      <div className="wf-row" style={{ borderTop: '1.2px solid var(--ink)', padding: '8px 16px', justifyContent: 'space-around', fontSize: 10, fontFamily: 'var(--mono)' }}>
        {['Drive', 'Photos', 'Shared', 'Me'].map((t, i) => (
          <div key={t} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2, color: i === 0 ? 'var(--ink)' : 'var(--ink-4)' }}>
            <Hex size={12} amber={i === 0} />
            <span>{t}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Android Drive ─────────────────────────────────────────────────────
function AndroidDrive({ variant = 'A' }) {
  return (
    <div style={{ background: 'var(--paper)', width: '100%', height: '100%', fontFamily: 'var(--sans)', color: 'var(--ink)', display: 'flex', flexDirection: 'column' }}>
      <div style={{ padding: '14px 16px 10px', borderBottom: '1.2px solid var(--ink)' }}>
        <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 6 }}>
          <Logo size={13} />
          <div className="wf-row" style={{ gap: 8 }}><span>⌕</span><span>⋮</span></div>
        </div>
        <JurisdictionBadge region="AMS" small />
      </div>

      {variant === 'A' && (
        <div style={{ flex: 1, padding: 0 }}>
          {[
            ['📄 Recents', '18 files', 'this week'],
            ['⭐ Starred', '4 files', ''],
            ['↗ Shared', '6 files', ''],
            ['⬡ Team · Brand', '412 files', 'FRA'],
            ['📷 Photos', '2,408', 'AMS'],
          ].map(([t, a, b], i) => (
            <div key={i} className="wf-row" style={{ padding: '14px 16px', borderBottom: '1px dashed rgba(26,26,26,0.2)', gap: 12 }}>
              <span style={{ flex: 1, fontSize: 13 }}>{t}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{a}</span>
              {b && <Chip style={{ fontSize: 9 }}>{b}</Chip>}
            </div>
          ))}
        </div>
      )}
      {variant === 'B' && (
        <div style={{ flex: 1, padding: 14 }}>
          <div style={{ fontFamily: 'var(--hand)', fontSize: 20, marginBottom: 10 }}>Quick access</div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
            {['Scan doc', 'Upload', 'New folder', 'Share'].map((l, i) => (
              <div key={l} className="wf-card" style={{ padding: 12, textAlign: 'center', background: i === 0 ? 'var(--amber-soft)' : 'var(--paper)' }}>
                <div style={{ fontSize: 18, marginBottom: 6 }}>⬡</div>
                <div style={{ fontSize: 11, fontWeight: 600 }}>{l}</div>
              </div>
            ))}
          </div>
          <div style={{ fontSize: 10, fontFamily: 'var(--mono)', textTransform: 'uppercase', color: 'var(--ink-3)', letterSpacing: 0.6, marginBottom: 8 }}>Recent</div>
          {['board-deck.pdf', 'term-sheet.docx', 'notes.md'].map((n, i) => (
            <div key={n} className="wf-row" style={{ padding: '8px 0', borderBottom: '1px dashed rgba(26,26,26,0.2)', gap: 10 }}>
              <span style={{ fontFamily: 'var(--mono)' }}>▤</span>
              <span style={{ fontSize: 12, flex: 1 }}>{n}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{i}h</span>
            </div>
          ))}
        </div>
      )}
      {variant === 'C' && (
        <div style={{ flex: 1, padding: 14, background: 'var(--paper-2)' }}>
          <div className="wf-card" style={{ padding: 14, marginBottom: 12, background: 'var(--amber-soft)' }}>
            <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-2)' }}>↑ uploading · 3 files</div>
            <div style={{ fontSize: 13, fontWeight: 600, margin: '4px 0' }}>designs.zip · 42%</div>
            <div style={{ height: 5, border: '1px solid var(--ink)' }}>
              <div style={{ width: '42%', height: '100%', background: 'var(--ink)' }} />
            </div>
          </div>
          <div style={{ fontSize: 10, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.6, marginBottom: 6 }}>Today</div>
          {['board-deck.pdf', 'photo-0421.heic', 'notes.md'].map(n => (
            <div key={n} className="wf-card" style={{ padding: 10, marginBottom: 8 }}>
              <div className="wf-row" style={{ gap: 10 }}>
                <span>▤</span><span style={{ flex: 1, fontSize: 12 }}>{n}</span><Chip style={{ fontSize: 9 }}>✓</Chip>
              </div>
            </div>
          ))}
        </div>
      )}
      {/* FAB */}
      <div style={{ position: 'absolute', bottom: 76, right: 20, width: 48, height: 48, background: 'var(--amber)', border: '1.2px solid var(--ink)', borderRadius: 24, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 20, boxShadow: '2px 2px 0 rgba(26,26,26,0.8)' }}>＋</div>
    </div>
  );
}

// ─── Desktop sync client (menu-bar popover) ────────────────────────────
function SyncTray({ variant = 'A' }) {
  return (
    <div className="wf-card" style={{ width: 320, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '10px 14px', borderBottom: '1.2px solid var(--ink)' }}>
        <Logo size={12} />
        <JurisdictionBadge region="FRA" small />
      </div>

      {variant === 'A' && (
        <>
          <div style={{ padding: 14 }}>
            <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', marginBottom: 4 }}>Status</div>
            <div style={{ fontFamily: 'var(--hand)', fontSize: 22, lineHeight: 1.1 }}>Syncing 3 files</div>
            <div style={{ height: 5, border: '1px solid var(--ink)', margin: '10px 0' }}>
              <div style={{ width: '58%', height: '100%', background: 'var(--amber)' }} />
            </div>
          </div>
          <Rule />
          <div style={{ padding: '6px 0' }}>
            {[['board-deck.pdf', '↑ uploading'], ['photo-0421.heic', '✓ synced'], ['designs.zip', '◉ encrypting']].map(([n, s], i) => (
              <div key={i} className="wf-row" style={{ padding: '6px 14px', gap: 10, fontSize: 11 }}>
                <span style={{ fontFamily: 'var(--mono)' }}>{s.split(' ')[0]}</span>
                <span style={{ flex: 1, fontFamily: 'var(--mono)' }}>{n}</span>
                <span style={{ fontFamily: 'var(--mono)', color: 'var(--ink-3)' }}>{s.split(' ')[1]}</span>
              </div>
            ))}
          </div>
          <Rule />
          <div className="wf-row" style={{ padding: '8px 14px', justifyContent: 'space-between', fontSize: 11 }}>
            <span style={{ fontFamily: 'var(--mono)', color: 'var(--ink-3)' }}>⬡ /Users/anna/Beebeeb</span>
            <span style={{ fontFamily: 'var(--mono)' }}>⚙</span>
          </div>
        </>
      )}

      {variant === 'B' && (
        <div style={{ padding: 14 }}>
          <div className="wf-row" style={{ gap: 10, marginBottom: 10 }}>
            <div style={{ width: 40, height: 46, clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)', background: 'var(--amber)', border: '1px solid var(--ink)' }} />
            <div>
              <div style={{ fontSize: 13, fontWeight: 600 }}>All caught up</div>
              <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>last sync · 14s ago</div>
            </div>
          </div>
          <Rule dashed style={{ margin: '10px 0' }} />
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, fontSize: 11 }}>
            <div><div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', textTransform: 'uppercase' }}>Uploaded</div>128 files</div>
            <div><div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', textTransform: 'uppercase' }}>Downloaded</div>44 files</div>
            <div><div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', textTransform: 'uppercase' }}>Region</div>Frankfurt</div>
            <div><div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', textTransform: 'uppercase' }}>Bandwidth</div>Unlimited</div>
          </div>
          <Btn alt small style={{ width: '100%', justifyContent: 'center', marginTop: 12 }}>Open folder</Btn>
        </div>
      )}

      {variant === 'C' && (
        <div style={{ padding: 14 }}>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', marginBottom: 6 }}>$ beebeeb status</div>
          <div style={{ fontFamily: 'var(--mono)', fontSize: 10, background: 'var(--paper-2)', border: '1px solid var(--ink)', padding: 10, lineHeight: 1.6 }}>
            <div>✓ daemon up · 3d 4h</div>
            <div>✓ region eu-fra-1</div>
            <div>↑ syncing 3/58 · 14 MB/s</div>
            <div>⬡ key fp: <span style={{ color: 'var(--amber-deep)' }}>a1b4·c7de·f204</span></div>
            <div style={{ color: 'var(--ink-3)' }}>---</div>
            <div>press q to quit</div>
          </div>
          <div className="wf-annot" style={{ marginTop: 10, fontSize: 13 }}>
            ↑ power-user mode for developers · open-source CLI
          </div>
        </div>
      )}
    </div>
  );
}

Object.assign(window, { IOSDrive, AndroidDrive, SyncTray });
