// hifi-ios-app.jsx — Wave 5: full iOS app screens

// Reuses PhoneShell from hifi-mobile-sync.jsx

// ─── Home / Files (expanded) ─────────────────────────────────────
function IOSHome() {
  const pinned = [
    { n: 'Ledger gap', t: 'folder', c: '23 files' },
    { n: 'Source docs', t: 'folder', c: '128 files' },
  ];
  const recent = [
    { n: 'story-draft.md', t: 'doc', d: '4m', dot: true },
    { n: 'interview-03.m4a', t: 'audio', d: '2h' },
    { n: 'leak-packet.pdf', t: 'pdf', d: '6h', warn: true },
    { n: 'photo-evidence/', t: 'folder', d: 'yest.' },
  ];
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '8px 18px 0' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
            <div style={{ width: 28, height: 28, borderRadius: '50%', background: 'linear-gradient(135deg, oklch(0.8 0.15 82), oklch(0.6 0.14 50))', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--paper)', fontSize: 11, fontWeight: 700 }}>IM</div>
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
              <div style={{ width: 30, height: 30, borderRadius: 999, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="search" size={13} color="var(--ink-2)" />
              </div>
              <div style={{ width: 30, height: 30, borderRadius: 999, background: 'var(--amber)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="plus" size={14} />
              </div>
            </div>
          </div>
          <div className="t-display" style={{ fontSize: 28, lineHeight: 1.05, marginBottom: 4 }}>Drive</div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 10 }}>
            <BBRegionBadge region="Frankfurt" />
            <span className="t-micro" style={{ color: 'var(--ink-3)' }}>· 23.4 / 200 GB</span>
          </div>
        </div>
        <div style={{ padding: '0 18px 10px' }}>
          <div className="t-label" style={{ marginBottom: 6, fontSize: 10 }}>Pinned</div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            {pinned.map((p, i) => (
              <div key={i} style={{
                padding: 10, borderRadius: 10, background: 'var(--amber-bg)',
                border: '1px solid oklch(0.88 0.06 88)',
              }}>
                <Ico name="folder" size={14} color="var(--amber-deep)" />
                <div style={{ fontSize: 12, fontWeight: 600, marginTop: 6 }}>{p.n}</div>
                <div className="t-mono" style={{ fontSize: 9.5, color: 'var(--ink-3)' }}>{p.c}</div>
              </div>
            ))}
          </div>
        </div>
        <div style={{ padding: '4px 18px 2px', display: 'flex', alignItems: 'center' }}>
          <div className="t-label" style={{ fontSize: 10 }}>Recent</div>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--amber-deep)' }}>See all</span>
        </div>
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {recent.map((f, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', gap: 11,
              padding: '10px 18px', borderBottom: '1px solid var(--line)',
            }}>
              <FileIconHi type={f.t} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 6 }}>
                  {f.n}
                  {f.dot && <span style={{ width: 5, height: 5, borderRadius: '50%', background: 'var(--amber-deep)' }} />}
                </div>
                <div className="t-mono" style={{ fontSize: 10, color: f.warn ? 'var(--red)' : 'var(--ink-3)' }}>
                  {f.warn ? 'One-time view · 47h left' : f.d}
                </div>
              </div>
              <Ico name="chevRight" size={12} color="var(--ink-4)" />
            </div>
          ))}
        </div>
        <IOSTabBar active="files" />
      </div>
    </PhoneShell>
  );
}

function IOSTabBar({ active = 'files' }) {
  const tabs = [
    ['files', 'Files', 'folder'],
    ['shared', 'Shared', 'users'],
    ['photos', 'Photos', 'image'],
    ['settings', 'Settings', 'settings'],
  ];
  return (
    <div style={{
      display: 'flex', padding: '8px 0 22px',
      borderTop: '1px solid var(--line)', background: 'var(--paper)',
    }}>
      {tabs.map(([id, l, ic]) => (
        <div key={id} style={{ flex: 1, textAlign: 'center', color: id === active ? 'var(--ink)' : 'var(--ink-4)' }}>
          <Ico name={ic} size={18} />
          <div style={{ fontSize: 9.5, marginTop: 2, fontWeight: id === active ? 600 : 400 }}>{l}</div>
        </div>
      ))}
    </div>
  );
}

// ─── Photos (grid) ─────────────────────────────────────
function IOSPhotos() {
  const months = [
    { label: 'September 2025', count: 42, warmTint: 0 },
    { label: 'August 2025', count: 127, warmTint: 1 },
  ];
  const swatch = (r, c) => {
    // deterministic warm gradient patches
    const hues = [55, 72, 28, 90, 42, 65, 18, 82];
    const lights = [0.78, 0.62, 0.84, 0.7, 0.66, 0.8];
    const h = hues[(r * 3 + c) % hues.length];
    const l = lights[(r + c) % lights.length];
    return `oklch(${l} 0.12 ${h})`;
  };
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '6px 18px 10px' }}>
          <div style={{ display: 'flex', alignItems: 'center' }}>
            <div className="t-display" style={{ fontSize: 24 }}>Photos</div>
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
              <div style={{ width: 28, height: 28, borderRadius: 999, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="search" size={12} color="var(--ink-2)" />
              </div>
              <div style={{ width: 28, height: 28, borderRadius: 999, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name="more" size={12} color="var(--ink-2)" />
              </div>
            </div>
          </div>
          <div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
            {['All', 'Years', 'Months', 'Days'].map((l, i) => (
              <div key={i} style={{
                padding: '4px 10px', borderRadius: 999, fontSize: 11,
                background: i === 2 ? 'var(--ink)' : 'var(--paper-2)',
                color: i === 2 ? 'var(--paper)' : 'var(--ink-3)',
                border: '1px solid', borderColor: i === 2 ? 'var(--ink)' : 'var(--line)',
                fontWeight: i === 2 ? 600 : 400,
              }}>{l}</div>
            ))}
          </div>
        </div>
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {months.map((m, mi) => (
            <div key={mi} style={{ marginBottom: 12 }}>
              <div style={{ padding: '0 18px 6px', display: 'flex', alignItems: 'baseline' }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>{m.label}</div>
                <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginLeft: 8 }}>{m.count} items</div>
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 2, padding: '0 2px' }}>
                {Array.from({ length: mi === 0 ? 12 : 8 }).map((_, i) => (
                  <div key={i} style={{
                    aspectRatio: '1', background: swatch(Math.floor(i / 4) + mi * 2, i % 4),
                    position: 'relative',
                  }}>
                    {i === 0 && mi === 0 && <div style={{ position: 'absolute', top: 4, right: 4, fontSize: 9, color: 'var(--paper)', background: 'rgba(0,0,0,0.4)', padding: '1px 4px', borderRadius: 3, fontFamily: 'var(--font-mono)' }}>0:47</div>}
                    {i === 5 && <div style={{ position: 'absolute', bottom: 4, right: 4, width: 10, height: 10, borderRadius: '50%', background: 'rgba(0,0,0,0.4)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                      <Ico name="users" size={6} color="var(--paper)" />
                    </div>}
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div style={{ padding: '8px 18px', background: 'var(--amber-bg)', borderTop: '1px solid oklch(0.88 0.06 88)', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Ico name="upload" size={11} color="var(--amber-deep)" />
          <span style={{ fontSize: 11, color: 'var(--ink-2)', flex: 1 }}>Auto-backup: <strong>3 new</strong></span>
          <span className="t-mono" style={{ fontSize: 10, color: 'var(--amber-deep)' }}>on Wi-Fi · 68%</span>
        </div>
        <IOSTabBar active="photos" />
      </div>
    </PhoneShell>
  );
}

// ─── File preview (iOS) ─────────────────────────────────────
function IOSPreview() {
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', background: '#0c0c0d', color: 'var(--paper)', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '6px 16px 10px', display: 'flex', alignItems: 'center', color: 'var(--paper)' }}>
          <Ico name="chevLeft" size={16} color="var(--paper)" />
          <div style={{ flex: 1, textAlign: 'center' }}>
            <div style={{ fontSize: 13, fontWeight: 600 }}>leak-packet.pdf</div>
            <div className="t-mono" style={{ fontSize: 9.5, opacity: 0.6 }}>3 of 24 · decrypting locally</div>
          </div>
          <Ico name="more" size={14} color="var(--paper)" />
        </div>
        <div style={{ flex: 1, padding: 14, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{
            width: '100%', aspectRatio: '0.72',
            background: 'oklch(0.96 0.005 85)', borderRadius: 4, color: 'var(--ink)',
            padding: '18px 16px', boxShadow: '0 8px 28px rgba(0,0,0,0.4)',
            display: 'flex', flexDirection: 'column', gap: 8,
          }}>
            <div style={{ fontFamily: 'Georgia, serif', fontSize: 13, fontWeight: 600 }}>AGREEMENT — CONFIDENTIAL</div>
            <div style={{ height: 1, background: 'var(--line-2)' }} />
            <div style={{ fontSize: 8, color: 'var(--ink-2)', lineHeight: 1.5, fontFamily: 'Georgia, serif' }}>
              This agreement, dated the third of March, is between the parties below and remains confidential under the provisions set forth in Schedule A.
            </div>
            <div style={{ fontSize: 8, color: 'var(--ink-3)', lineHeight: 1.5, fontFamily: 'Georgia, serif' }}>
              1. The parties hereby agree to the terms set forth. 2. Consideration shall be deemed delivered upon signature. 3. All disputes arising from
            </div>
            <div style={{ height: 1, background: 'var(--line)', margin: '4px 0' }} />
            <div style={{ fontSize: 8, color: 'var(--ink-3)', lineHeight: 1.5, fontFamily: 'Georgia, serif', background: 'oklch(0.95 0.04 82)', padding: 4, borderLeft: '2px solid var(--amber-deep)' }}>
              Highlighted: "…total consideration shall not exceed four million…"
            </div>
            <div style={{ flex: 1 }} />
            <div className="t-mono" style={{ fontSize: 8, color: 'var(--ink-4)', marginTop: 'auto' }}>Page 3 / 24 · redacted</div>
          </div>
        </div>
        <div style={{
          padding: '12px 16px 22px', background: 'rgba(20,20,22,0.9)',
          borderTop: '1px solid rgba(255,255,255,0.08)',
          display: 'flex', justifyContent: 'space-around', alignItems: 'center',
        }}>
          {[['share', 'Share'], ['star', 'Pin'], ['folder', 'Move'], ['trash', 'Delete'], ['more', 'More']].map(([ic, l], i) => (
            <div key={i} style={{ textAlign: 'center', color: 'var(--paper)', opacity: 0.85 }}>
              <Ico name={ic} size={16} color="var(--paper)" />
              <div style={{ fontSize: 9.5, marginTop: 4 }}>{l}</div>
            </div>
          ))}
        </div>
      </div>
    </PhoneShell>
  );
}

// ─── Share sheet (bottom sheet) ─────────────────────────────────────
function IOSShareSheet() {
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', position: 'relative', background: 'rgba(0,0,0,0.35)' }}>
        {/* dimmed background: show the home behind at ~40% opacity */}
        <div style={{ position: 'absolute', inset: 0, opacity: 0.35, pointerEvents: 'none' }}>
          <div style={{ padding: '8px 18px' }}>
            <div className="t-display" style={{ fontSize: 28 }}>Drive</div>
          </div>
        </div>
        <div style={{
          position: 'absolute', left: 8, right: 8, bottom: 8,
          background: 'var(--paper)', borderRadius: 20,
          boxShadow: '0 -10px 30px rgba(0,0,0,0.25)',
          padding: '12px 16px 24px',
        }}>
          <div style={{ width: 36, height: 4, borderRadius: 2, background: 'var(--line-2)', margin: '0 auto 14px' }} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
            <FileIconHi type="doc" />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>fact-check-notes.pdf</div>
              <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>2.4 MB · encrypted</div>
            </div>
          </div>

          <div className="t-label" style={{ marginBottom: 8 }}>Share via</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6, marginBottom: 14 }}>
            {[
              { ic: 'share', l: 'Link', bg: 'var(--amber-bg)', c: 'var(--amber-deep)' },
              { ic: 'users', l: 'Beebeeb', bg: 'var(--paper-2)', c: 'var(--ink)' },
              { ic: 'key', l: 'Passphrase', bg: 'var(--paper-2)', c: 'var(--ink)' },
              { ic: 'more', l: 'Other', bg: 'var(--paper-2)', c: 'var(--ink)' },
            ].map((a, i) => (
              <div key={i} style={{ textAlign: 'center' }}>
                <div style={{
                  width: 44, height: 44, borderRadius: 12, background: a.bg, color: a.c,
                  display: 'flex', alignItems: 'center', justifyContent: 'center', margin: '0 auto 4px',
                  border: a.bg === 'var(--paper-2)' ? '1px solid var(--line)' : '1px solid oklch(0.88 0.07 88)',
                }}>
                  <Ico name={a.ic} size={16} color={a.c} />
                </div>
                <div style={{ fontSize: 10, color: 'var(--ink-2)' }}>{a.l}</div>
              </div>
            ))}
          </div>

          <div style={{ background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 12, padding: 10, marginBottom: 10 }}>
            <div className="t-label" style={{ marginBottom: 6, fontSize: 10 }}>Link settings</div>
            {[
              { l: 'Require passphrase', v: 'Set', dot: 'var(--amber-deep)' },
              { l: 'Expires', v: 'in 5 days' },
              { l: 'Max opens', v: '1 · one-time' },
              { l: 'Allow download', v: 'Off' },
            ].map((r, i, arr) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', padding: '6px 0', borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none' }}>
                <span style={{ fontSize: 12, flex: 1 }}>{r.l}</span>
                <span className="t-mono" style={{ fontSize: 11, color: r.dot || 'var(--ink-3)' }}>{r.v}</span>
                <Ico name="chevRight" size={10} color="var(--ink-4)" />
              </div>
            ))}
          </div>

          <BBBtn variant="primary" style={{ width: '100%', padding: '11px 16px', fontSize: 13, justifyContent: 'center', borderRadius: 12 }}>
            Create encrypted link
          </BBBtn>
        </div>
      </div>
    </PhoneShell>
  );
}

// ─── Settings (iOS) ─────────────────────────────────────
function IOSSettings() {
  const groups = [
    {
      items: [
        { ic: 'users', l: 'Isa Marchetti', sub: 'isa@example.com · Team · 23.4 / 200 GB', hero: true },
      ],
    },
    {
      label: 'Security',
      items: [
        { ic: 'lock', l: 'Biometrics', v: 'Face ID' },
        { ic: 'shield', l: 'Two-factor', v: 'Passkey + backup' },
        { ic: 'key', l: 'Recovery phrase', v: 'Exported 4 weeks ago' },
        { ic: 'cloud', l: 'Devices', v: '5' },
        { ic: 'lock', l: 'Lock app on leave', v: 'Immediately', chev: true },
      ],
    },
    {
      label: 'Backup',
      items: [
        { ic: 'image', l: 'Camera roll', v: 'On · Wi-Fi only' },
        { ic: 'upload', l: 'Background upload', toggle: true },
        { ic: 'folder', l: 'Files app integration', toggle: true },
      ],
    },
    {
      label: 'App',
      items: [
        { ic: 'settings', l: 'Appearance', v: 'System' },
        { ic: 'star', l: 'Language', v: 'English' },
        { ic: 'clock', l: 'Clear local cache', v: '1.4 GB' },
      ],
    },
    {
      items: [
        { ic: 'lock', l: 'Sign out', danger: true },
      ],
    },
  ];
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--paper-2)' }}>
        <div style={{ padding: '6px 18px 10px', background: 'var(--paper-2)' }}>
          <div className="t-display" style={{ fontSize: 24 }}>Settings</div>
        </div>
        <div style={{ flex: 1, overflow: 'hidden', padding: '0 14px' }}>
          {groups.map((g, gi) => (
            <div key={gi} style={{ marginBottom: 14 }}>
              {g.label && <div className="t-label" style={{ fontSize: 10, padding: '0 6px 6px' }}>{g.label}</div>}
              <div style={{ background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line)', overflow: 'hidden' }}>
                {g.items.map((it, i, arr) => (
                  <div key={i} style={{
                    display: 'flex', alignItems: 'center', gap: 10,
                    padding: it.hero ? '12px 12px' : '9px 12px',
                    borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                  }}>
                    {it.hero ? (
                      <div style={{ width: 34, height: 34, borderRadius: '50%', background: 'linear-gradient(135deg, oklch(0.8 0.15 82), oklch(0.6 0.14 50))', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--paper)', fontSize: 12, fontWeight: 700 }}>IM</div>
                    ) : (
                      <div style={{ width: 22, height: 22, borderRadius: 5, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                        <Ico name={it.ic} size={11} color={it.danger ? 'var(--red)' : 'var(--ink-2)'} />
                      </div>
                    )}
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 13, fontWeight: it.hero ? 600 : 500, color: it.danger ? 'var(--red)' : 'var(--ink)' }}>{it.l}</div>
                      {it.sub && <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginTop: 2 }}>{it.sub}</div>}
                    </div>
                    {it.v && <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.v}</span>}
                    {it.toggle && <BBToggle on />}
                    {(it.v || it.chev || it.hero) && !it.toggle && !it.danger && <Ico name="chevRight" size={11} color="var(--ink-4)" />}
                  </div>
                ))}
              </div>
            </div>
          ))}
          <div style={{ textAlign: 'center', fontSize: 10, color: 'var(--ink-4)', paddingBottom: 10 }}>
            <span className="t-mono">v1.2.0 · build 442</span><br />
            <span style={{ color: 'var(--amber-deep)' }}>beebeeb.io · Frankfurt</span>
          </div>
        </div>
        <IOSTabBar active="settings" />
      </div>
    </PhoneShell>
  );
}

// ─── Biometric unlock / lock screen ─────────────────────────────────────
function IOSBiometricLock() {
  return (
    <PhoneShell platform="ios">
      <div style={{
        height: '100%', display: 'flex', flexDirection: 'column',
        background: 'radial-gradient(120% 80% at 50% 40%, oklch(0.98 0.03 82) 0%, var(--paper) 60%, oklch(0.92 0.04 70) 100%)',
      }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', alignItems: 'center', padding: 24 }}>
          <div style={{ transform: 'scale(1.4)', marginBottom: 22 }}>
            <BBLogo size={18} />
          </div>
          <div className="t-display" style={{ fontSize: 22, textAlign: 'center', marginBottom: 6 }}>Beebeeb is locked</div>
          <div style={{ fontSize: 12.5, color: 'var(--ink-3)', textAlign: 'center', maxWidth: 240, lineHeight: 1.5 }}>
            Your vault key lives only on this device. Use Face ID to unlock it locally — nothing leaves the phone.
          </div>

          <div style={{ marginTop: 40, display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
            <div style={{
              width: 80, height: 80, borderRadius: 22,
              background: 'var(--paper)', border: '2px solid var(--ink)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              boxShadow: '0 0 0 8px oklch(0.96 0.05 82), 0 0 0 9px oklch(0.88 0.07 80)',
              position: 'relative',
            }}>
              <svg width="40" height="40" viewBox="0 0 40 40">
                <circle cx="20" cy="20" r="14" stroke="var(--ink)" strokeWidth="2" fill="none" />
                <circle cx="15" cy="17" r="1.5" fill="var(--ink)" />
                <circle cx="25" cy="17" r="1.5" fill="var(--ink)" />
                <path d="M14 24 Q20 28 26 24" stroke="var(--ink)" strokeWidth="2" fill="none" strokeLinecap="round" />
              </svg>
            </div>
            <div style={{ marginTop: 14, fontSize: 12, color: 'var(--ink-3)' }}>Look to unlock</div>
          </div>
        </div>

        <div style={{ padding: '16px 24px 28px', textAlign: 'center' }}>
          <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>Or <span style={{ color: 'var(--amber-deep)', fontWeight: 500 }}>use master passphrase</span></div>
          <div className="t-mono" style={{ fontSize: 9.5, color: 'var(--ink-4)', marginTop: 10 }}>
            Locked 2m ago · auto-lock after 1m
          </div>
        </div>
      </div>
    </PhoneShell>
  );
}

// ─── Auto-backup detail ─────────────────────────────────────
function IOSAutoBackup() {
  return (
    <PhoneShell platform="ios">
      <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '4px 16px 0', display: 'flex', alignItems: 'center' }}>
          <Ico name="chevLeft" size={16} />
          <div style={{ flex: 1, textAlign: 'center', fontSize: 13, fontWeight: 600 }}>Camera backup</div>
          <BBToggle on />
        </div>

        <div style={{ padding: '16px 18px 8px' }}>
          <div style={{
            padding: 16, borderRadius: 14,
            background: 'linear-gradient(135deg, var(--ink) 0%, oklch(0.22 0.008 80) 100%)',
            color: 'var(--paper)', position: 'relative', overflow: 'hidden',
          }}>
            <svg style={{ position: 'absolute', right: -10, top: -10, opacity: 0.12 }} width="120" height="120" viewBox="0 0 120 120" fill="none">
              <polygon points="60,10 100,32 100,78 60,100 20,78 20,32" stroke="var(--amber)" strokeWidth="2" fill="none" />
              <polygon points="60,30 82,42 82,66 60,78 38,66 38,42" stroke="var(--amber)" strokeWidth="1.5" fill="none" />
            </svg>
            <div style={{ fontSize: 11, opacity: 0.6, marginBottom: 6 }}>THIS PHONE</div>
            <div className="t-display" style={{ fontSize: 26, color: 'var(--paper)' }}>1,247</div>
            <div style={{ fontSize: 12, opacity: 0.8, marginTop: 2 }}>photos encrypted &amp; backed up</div>
            <div style={{ display: 'flex', gap: 14, marginTop: 12, fontSize: 11, opacity: 0.85 }}>
              <div>3.4 GB</div>
              <div>·</div>
              <div>3 pending</div>
            </div>
          </div>
        </div>

        <div style={{ padding: '8px 18px' }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Currently backing up</div>
          <div style={{ background: 'var(--amber-bg)', border: '1px solid oklch(0.88 0.07 88)', borderRadius: 10, padding: 10 }}>
            {[
              { n: 'IMG_4829.HEIC', p: 87, sz: '2.4 MB' },
              { n: 'IMG_4830.HEIC', p: 0, sz: '3.1 MB' },
              { n: 'IMG_4831.MOV', p: 0, sz: '18 MB' },
            ].map((f, i, arr) => (
              <div key={i} style={{ padding: '5px 0', borderBottom: i < arr.length - 1 ? '1px solid oklch(0.88 0.07 88)' : 'none' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Ico name="image" size={11} color="var(--amber-deep)" />
                  <span className="t-mono" style={{ fontSize: 11, flex: 1 }}>{f.n}</span>
                  <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>
                    {f.p > 0 ? `${f.p}%` : 'queued'}
                  </span>
                </div>
                {f.p > 0 && <div className="bb-progress" style={{ marginTop: 4, height: 2 }}>
                  <div style={{ width: `${f.p}%` }} />
                </div>}
              </div>
            ))}
          </div>
        </div>

        <div style={{ padding: '10px 14px' }}>
          <div style={{ background: 'var(--paper)', border: '1px solid var(--line)', borderRadius: 12, overflow: 'hidden' }}>
            {[
              { l: 'Only on Wi-Fi', toggle: true },
              { l: 'Only while charging', toggle: false },
              { l: 'Back up Live Photos', toggle: true },
              { l: 'Back up videos', v: 'Up to 500 MB' },
              { l: 'Delete local after upload', toggle: false, hint: 'Keeps 30 days' },
            ].map((r, i, arr) => (
              <div key={i} style={{
                padding: '10px 12px', borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                display: 'flex', alignItems: 'center', gap: 8,
              }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13 }}>{r.l}</div>
                  {r.hint && <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 1 }}>{r.hint}</div>}
                </div>
                {r.toggle !== undefined && <BBToggle on={r.toggle} />}
                {r.v && <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r.v}</span>}
              </div>
            ))}
          </div>
        </div>
      </div>
    </PhoneShell>
  );
}

Object.assign(window, {
  IOSHome, IOSPhotos, IOSPreview, IOSShareSheet, IOSSettings, IOSBiometricLock, IOSAutoBackup, IOSTabBar,
});
