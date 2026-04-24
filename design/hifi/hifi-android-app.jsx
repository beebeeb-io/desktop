// hifi-android-app.jsx — Wave 6: full Android app screens (Material-ish)

// ─── Android Home / Files ─────────────────────────────────────
function AndHome() {
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--paper)' }}>
        {/* App bar */}
        <div style={{ padding: '12px 16px 8px', display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ width: 28, height: 28, borderRadius: '50%', background: 'linear-gradient(135deg, oklch(0.8 0.15 82), oklch(0.6 0.14 50))', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--paper)', fontSize: 10, fontWeight: 700 }}>IM</div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 10, color: 'var(--ink-3)' }}>Good evening</div>
            <div style={{ fontSize: 14, fontWeight: 600 }}>Isa</div>
          </div>
          <Ico name="search" size={16} color="var(--ink-2)" />
          <Ico name="more" size={16} color="var(--ink-2)" />
        </div>

        {/* Storage card */}
        <div style={{ padding: '0 14px 10px' }}>
          <div style={{
            padding: 14, borderRadius: 16,
            background: 'linear-gradient(135deg, var(--ink) 0%, oklch(0.22 0.008 80) 100%)',
            color: 'var(--paper)', position: 'relative', overflow: 'hidden',
          }}>
            <svg style={{ position: 'absolute', right: -18, top: -18, opacity: 0.1 }} width="110" height="110" viewBox="0 0 120 120" fill="none">
              <polygon points="60,10 100,32 100,78 60,100 20,78 20,32" stroke="var(--amber)" strokeWidth="2" fill="none" />
            </svg>
            <div style={{ fontSize: 10, opacity: 0.6, letterSpacing: 0.3 }}>FRANKFURT VAULT · E2EE</div>
            <div className="t-display" style={{ fontSize: 22, marginTop: 4, color: 'var(--paper)' }}>23.4 GB</div>
            <div style={{ fontSize: 11, opacity: 0.75, marginTop: 2 }}>of 200 GB · Team plan</div>
            <div style={{ marginTop: 10, height: 4, background: 'rgba(255,255,255,0.12)', borderRadius: 2, overflow: 'hidden' }}>
              <div style={{ width: '12%', height: '100%', background: 'var(--amber)' }} />
            </div>
          </div>
        </div>

        {/* Quick tiles */}
        <div style={{ padding: '0 14px 12px', display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
          {[
            { ic: 'upload', l: 'Upload' },
            { ic: 'image', l: 'Scan' },
            { ic: 'folder', l: 'New folder' },
            { ic: 'share', l: 'Quick send' },
          ].map((a, i) => (
            <div key={i} style={{
              padding: '10px 4px', borderRadius: 12,
              background: 'var(--paper-2)', border: '1px solid var(--line)',
              display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 5,
            }}>
              <Ico name={a.ic} size={14} color="var(--amber-deep)" />
              <div style={{ fontSize: 10, fontWeight: 500 }}>{a.l}</div>
            </div>
          ))}
        </div>

        {/* Recent list */}
        <div style={{ padding: '0 14px 8px', display: 'flex' }}>
          <div className="t-label" style={{ fontSize: 10 }}>Recent</div>
          <span style={{ marginLeft: 'auto', fontSize: 10.5, color: 'var(--amber-deep)', fontWeight: 500 }}>See all</span>
        </div>
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {[
            { n: 'story-draft.md', t: 'doc', d: '4 min ago' },
            { n: 'Ledger gap/', t: 'folder', d: '2h · 23 files' },
            { n: 'interview-03.m4a', t: 'audio', d: 'today 14:22' },
            { n: 'leak-packet.pdf', t: 'pdf', d: '1-time · 47h' },
          ].map((f, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', gap: 12,
              padding: '10px 16px', borderTop: i === 0 ? '1px solid var(--line)' : 'none',
              borderBottom: '1px solid var(--line)',
            }}>
              <FileIconHi type={f.t} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500 }}>{f.n}</div>
                <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>{f.d}</div>
              </div>
              <Ico name="more" size={13} color="var(--ink-4)" />
            </div>
          ))}
        </div>

        {/* FAB */}
        <div style={{ position: 'absolute', right: 20, bottom: 80, zIndex: 5 }}>
          <div style={{
            width: 52, height: 52, borderRadius: 16,
            background: 'var(--amber)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            boxShadow: '0 6px 16px rgba(0,0,0,0.18)',
          }}>
            <Ico name="plus" size={20} color="var(--ink)" />
          </div>
        </div>

        <AndNav active="files" />
      </div>
    </PhoneShell>
  );
}

function AndNav({ active }) {
  const tabs = [
    ['files', 'Files', 'folder'],
    ['shared', 'Shared', 'users'],
    ['photos', 'Photos', 'image'],
    ['me', 'You', 'settings'],
  ];
  return (
    <div style={{ display: 'flex', padding: '6px 0 10px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
      {tabs.map(([id, l, ic]) => {
        const a = id === active;
        return (
          <div key={id} style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 3, padding: '4px 0' }}>
            <div style={{
              padding: '4px 16px', borderRadius: 999,
              background: a ? 'var(--amber-bg)' : 'transparent',
            }}>
              <Ico name={ic} size={15} color={a ? 'var(--amber-deep)' : 'var(--ink-3)'} />
            </div>
            <span style={{ fontSize: 10, color: a ? 'var(--ink)' : 'var(--ink-3)', fontWeight: a ? 600 : 400 }}>{l}</span>
          </div>
        );
      })}
    </div>
  );
}

// ─── Android Preview ─────────────────────────────────────
function AndPreview() {
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: '#0c0c0d' }}>
        <div style={{ padding: '12px 14px', display: 'flex', alignItems: 'center', gap: 10, color: 'var(--paper)' }}>
          <Ico name="chevLeft" size={16} color="var(--paper)" />
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 13, fontWeight: 600 }}>leak-packet.pdf</div>
            <div className="t-mono" style={{ fontSize: 9.5, opacity: 0.6 }}>decrypted locally</div>
          </div>
          <Ico name="share" size={14} color="var(--paper)" />
          <Ico name="more" size={14} color="var(--paper)" />
        </div>

        <div style={{ flex: 1, padding: '8px 14px', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{
            width: '100%', aspectRatio: '0.72',
            background: 'oklch(0.96 0.005 85)', borderRadius: 4, color: 'var(--ink)',
            padding: '18px 16px', boxShadow: '0 8px 28px rgba(0,0,0,0.4)',
            display: 'flex', flexDirection: 'column', gap: 7,
          }}>
            <div style={{ fontFamily: 'Georgia, serif', fontSize: 12, fontWeight: 600 }}>INTERNAL MEMO — NOT FOR DISTRIBUTION</div>
            <div style={{ height: 1, background: 'var(--line-2)' }} />
            <div style={{ fontSize: 8, lineHeight: 1.5, color: 'var(--ink-2)', fontFamily: 'Georgia, serif' }}>
              Subject: Quarterly reconciliation review. The attached schedule indicates a gap of approximately 4.2M between reported figures and bank records.
            </div>
            <div style={{ fontSize: 8, lineHeight: 1.5, color: 'var(--ink-3)', fontFamily: 'Georgia, serif' }}>
              Cross-reference: see Exhibit B and the letter dated 14 March. Forward to counsel before any external comment.
            </div>
            <div style={{ height: 1, background: 'var(--line)', margin: '3px 0' }} />
            <div style={{ fontSize: 8, color: 'var(--ink-3)', fontFamily: 'Georgia, serif', background: 'oklch(0.95 0.04 82)', padding: 4, borderLeft: '2px solid var(--amber-deep)' }}>
              "…gap of approximately 4.2M…"
            </div>
            <div style={{ flex: 1 }} />
            <div className="t-mono" style={{ fontSize: 8, color: 'var(--ink-4)' }}>p. 3 / 24 · watermark: isa@example.com</div>
          </div>
        </div>

        <div style={{
          padding: '10px 14px 14px', background: 'rgba(20,20,22,0.9)',
          display: 'flex', justifyContent: 'space-around', alignItems: 'center',
        }}>
          {[['star', 'Pin'], ['folder', 'Move'], ['share', 'Share'], ['trash', 'Delete']].map(([ic, l], i) => (
            <div key={i} style={{ textAlign: 'center', color: 'var(--paper)', opacity: 0.85 }}>
              <Ico name={ic} size={15} color="var(--paper)" />
              <div style={{ fontSize: 9.5, marginTop: 4 }}>{l}</div>
            </div>
          ))}
        </div>
      </div>
    </PhoneShell>
  );
}

// ─── Android Photos ─────────────────────────────────────
function AndPhotos() {
  const swatch = (i) => {
    const hues = [55, 72, 28, 90, 42, 65, 18, 82];
    const lights = [0.78, 0.62, 0.84, 0.7, 0.66, 0.8];
    return `oklch(${lights[i % lights.length]} 0.12 ${hues[i % hues.length]})`;
  };
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '12px 16px 8px', display: 'flex', alignItems: 'center' }}>
          <div>
            <div className="t-display" style={{ fontSize: 22 }}>Photos</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>1,247 encrypted · 3 pending</div>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <Ico name="search" size={15} color="var(--ink-2)" />
            <Ico name="more" size={15} color="var(--ink-2)" />
          </div>
        </div>

        <div style={{ padding: '0 14px 8px', display: 'flex', gap: 6 }}>
          {['Timeline', 'Albums', 'Favorites'].map((l, i) => (
            <div key={i} style={{
              padding: '5px 12px', borderRadius: 999, fontSize: 11,
              background: i === 0 ? 'var(--ink)' : 'var(--paper-2)',
              color: i === 0 ? 'var(--paper)' : 'var(--ink-3)',
              border: '1px solid', borderColor: i === 0 ? 'var(--ink)' : 'var(--line)',
              fontWeight: i === 0 ? 600 : 400,
            }}>{l}</div>
          ))}
        </div>

        <div style={{ flex: 1, overflow: 'hidden' }}>
          <div style={{ padding: '6px 16px 4px', display: 'flex', alignItems: 'baseline', gap: 8 }}>
            <div style={{ fontSize: 12.5, fontWeight: 600 }}>Today</div>
            <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>Sep 24 · 7 items</div>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 2 }}>
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} style={{ aspectRatio: '1', background: swatch(i), position: 'relative' }}>
                {i === 2 && <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <div style={{ width: 22, height: 22, borderRadius: '50%', background: 'rgba(0,0,0,0.5)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    <div style={{ width: 0, height: 0, borderLeft: '6px solid var(--paper)', borderTop: '4px solid transparent', borderBottom: '4px solid transparent', marginLeft: 2 }} />
                  </div>
                </div>}
              </div>
            ))}
          </div>

          <div style={{ padding: '10px 16px 4px', display: 'flex', alignItems: 'baseline', gap: 8 }}>
            <div style={{ fontSize: 12.5, fontWeight: 600 }}>September</div>
            <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>42 items</div>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 2 }}>
            {Array.from({ length: 9 }).map((_, i) => (
              <div key={i} style={{ aspectRatio: '1', background: swatch(i + 6) }} />
            ))}
          </div>
        </div>

        <div style={{
          margin: '8px 14px', padding: '10px 12px',
          background: 'var(--amber-bg)', border: '1px solid oklch(0.88 0.07 88)',
          borderRadius: 14, display: 'flex', alignItems: 'center', gap: 10,
        }}>
          <Ico name="upload" size={13} color="var(--amber-deep)" />
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 11.5, fontWeight: 600 }}>3 items backing up</div>
            <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>Wi-Fi · 68%</div>
          </div>
          <div className="bb-progress" style={{ width: 50, height: 3 }}><div style={{ width: '68%' }} /></div>
        </div>

        <AndNav active="photos" />
      </div>
    </PhoneShell>
  );
}

// ─── Android Settings ─────────────────────────────────────
function AndSettings() {
  const groups = [
    {
      label: 'Account',
      items: [
        { ic: 'users', l: 'Profile', v: 'Isa M.' },
        { ic: 'cloud', l: 'Storage', v: '23.4 / 200 GB' },
        { ic: 'shield', l: 'Security center', v: 'Strong' },
      ],
    },
    {
      label: 'Data',
      items: [
        { ic: 'image', l: 'Camera backup', v: 'On · Wi-Fi' },
        { ic: 'folder', l: 'Selected folders', v: '4 active' },
        { ic: 'upload', l: 'Background upload', toggle: true },
      ],
    },
    {
      label: 'App',
      items: [
        { ic: 'lock', l: 'App lock', v: 'Fingerprint · 1m' },
        { ic: 'settings', l: 'Theme', v: 'System' },
        { ic: 'star', l: 'Language', v: 'English' },
        { ic: 'clock', l: 'Clear cache', v: '1.4 GB' },
      ],
    },
    {
      items: [
        { ic: 'lock', l: 'Sign out', danger: true },
      ],
    },
  ];
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--paper-2)' }}>
        <div style={{ padding: '14px 16px 6px', background: 'var(--paper-2)' }}>
          <div className="t-display" style={{ fontSize: 22 }}>You</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>isa@example.com · Team plan</div>
        </div>
        <div style={{ flex: 1, overflow: 'hidden', padding: '4px 12px 8px' }}>
          {groups.map((g, gi) => (
            <div key={gi} style={{ marginTop: 10 }}>
              {g.label && <div className="t-label" style={{ fontSize: 10, padding: '0 4px 6px' }}>{g.label}</div>}
              <div style={{ background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line)', overflow: 'hidden' }}>
                {g.items.map((it, i, arr) => (
                  <div key={i} style={{
                    display: 'flex', alignItems: 'center', gap: 12,
                    padding: '11px 14px',
                    borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                  }}>
                    <div style={{
                      width: 30, height: 30, borderRadius: '50%',
                      background: it.danger ? 'oklch(0.94 0.05 25)' : 'var(--paper-2)',
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                    }}>
                      <Ico name={it.ic} size={13} color={it.danger ? 'var(--red)' : 'var(--ink-2)'} />
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 13, fontWeight: 500, color: it.danger ? 'var(--red)' : 'var(--ink)' }}>{it.l}</div>
                    </div>
                    {it.v && <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.v}</span>}
                    {it.toggle && <BBToggle on />}
                  </div>
                ))}
              </div>
            </div>
          ))}
          <div style={{ textAlign: 'center', fontSize: 10, color: 'var(--ink-4)', padding: '12px 0 6px' }}>
            <span className="t-mono">v1.2.0 · build 442</span> · <span style={{ color: 'var(--amber-deep)' }}>Frankfurt</span>
          </div>
        </div>
        <AndNav active="me" />
      </div>
    </PhoneShell>
  );
}

// ─── Android Share Sheet ─────────────────────────────────────
function AndShareSheet() {
  return (
    <PhoneShell platform="android">
      <div style={{ height: '100%', position: 'relative', background: 'rgba(0,0,0,0.35)' }}>
        <div style={{ position: 'absolute', inset: 0, opacity: 0.3, pointerEvents: 'none' }}>
          <div style={{ padding: '12px 16px' }}>
            <div className="t-display" style={{ fontSize: 22 }}>Files</div>
          </div>
        </div>
        <div style={{
          position: 'absolute', left: 6, right: 6, bottom: 6,
          background: 'var(--paper)', borderRadius: 22,
          boxShadow: '0 -10px 30px rgba(0,0,0,0.22)',
          padding: '10px 16px 22px',
        }}>
          <div style={{ width: 36, height: 4, borderRadius: 2, background: 'var(--line-2)', margin: '0 auto 12px' }} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
            <FileIconHi type="pdf" />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>board-deck-apr.pdf</div>
              <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>4.2 MB · will encrypt before share</div>
            </div>
          </div>

          <div className="t-label" style={{ marginBottom: 8 }}>Send with</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 6, marginBottom: 14 }}>
            {[
              { ic: 'share', l: 'Link', amber: true },
              { ic: 'users', l: 'Direct' },
              { ic: 'key', l: 'Pass' },
              { ic: 'cloud', l: 'Vault' },
              { ic: 'more', l: 'More' },
            ].map((a, i) => (
              <div key={i} style={{ textAlign: 'center' }}>
                <div style={{
                  width: 40, height: 40, borderRadius: '50%',
                  background: a.amber ? 'var(--amber-bg)' : 'var(--paper-2)',
                  border: '1px solid', borderColor: a.amber ? 'oklch(0.88 0.07 88)' : 'var(--line)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', margin: '0 auto 4px',
                }}>
                  <Ico name={a.ic} size={14} color={a.amber ? 'var(--amber-deep)' : 'var(--ink-2)'} />
                </div>
                <div style={{ fontSize: 10, color: 'var(--ink-2)' }}>{a.l}</div>
              </div>
            ))}
          </div>

          <div style={{ background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 14, padding: 10, marginBottom: 10 }}>
            {[
              { l: 'Passphrase', v: 'Auto · 6 words', dot: true },
              { l: 'Expires', v: 'in 24h' },
              { l: 'Download', v: 'Off · view only' },
            ].map((r, i, arr) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', padding: '7px 0', borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none' }}>
                <span style={{ fontSize: 12, flex: 1 }}>{r.l}</span>
                <span className="t-mono" style={{ fontSize: 11, color: r.dot ? 'var(--amber-deep)' : 'var(--ink-3)' }}>{r.v}</span>
                <Ico name="chevRight" size={10} color="var(--ink-4)" />
              </div>
            ))}
          </div>

          <BBBtn variant="primary" style={{ width: '100%', padding: '12px 16px', fontSize: 13, justifyContent: 'center', borderRadius: 999 }}>
            Create encrypted link
          </BBBtn>
        </div>
      </div>
    </PhoneShell>
  );
}

// ─── Android Fingerprint Lock ─────────────────────────────────────
function AndFingerprint() {
  return (
    <PhoneShell platform="android">
      <div style={{
        height: '100%', display: 'flex', flexDirection: 'column',
        background: 'radial-gradient(120% 80% at 50% 30%, oklch(0.98 0.02 82) 0%, var(--paper) 60%)',
      }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', alignItems: 'center', padding: 24 }}>
          <BBLogo size={16} />
          <div className="t-display" style={{ fontSize: 20, marginTop: 14, textAlign: 'center' }}>Beebeeb is locked</div>
          <div style={{ fontSize: 12, color: 'var(--ink-3)', textAlign: 'center', maxWidth: 240, lineHeight: 1.5, marginTop: 6 }}>
            Vault key stays on this phone. Biometrics unlock it locally — nothing transmitted.
          </div>

          <div style={{
            marginTop: 42, width: 74, height: 74, borderRadius: '50%',
            background: 'var(--paper)', border: '2px solid var(--amber-deep)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            boxShadow: '0 0 0 6px oklch(0.96 0.05 82)',
          }}>
            <svg width="40" height="40" viewBox="0 0 40 40" fill="none">
              {[14, 10, 6, 2].map((r, i) => (
                <ellipse key={i} cx="20" cy="20" rx={r} ry={r + 2} stroke="var(--amber-deep)" strokeWidth="1.5" fill="none" strokeDasharray={i % 2 ? '6 4' : 'none'} />
              ))}
            </svg>
          </div>
          <div style={{ marginTop: 14, fontSize: 12, color: 'var(--amber-deep)', fontWeight: 500 }}>Touch sensor to unlock</div>
        </div>

        <div style={{ padding: '16px 24px 28px', textAlign: 'center' }}>
          <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>Or <span style={{ color: 'var(--amber-deep)', fontWeight: 500 }}>use PIN</span></div>
          <div className="t-mono" style={{ fontSize: 9.5, color: 'var(--ink-4)', marginTop: 10 }}>Auto-lock after 1m</div>
        </div>
      </div>
    </PhoneShell>
  );
}

Object.assign(window, { AndHome, AndPreview, AndPhotos, AndSettings, AndShareSheet, AndFingerprint });
