// hifi-mobile-extra.jsx — offline indicator, in-app upgrade, share extension, push notifications

function MobileShell({ children, platform = 'ios', width = 320, height = 660 }) {
  return (
    <div style={{
      width, height,
      borderRadius: platform === 'ios' ? 36 : 24,
      background: 'var(--ink)',
      padding: 7,
      boxShadow: '0 40px 80px -40px rgba(0,0,0,0.3)',
      margin: '0 auto'
    }}>
      <div style={{
        width: '100%', height: '100%',
        borderRadius: platform === 'ios' ? 30 : 18,
        background: 'var(--paper)',
        overflow: 'hidden',
        display: 'flex', flexDirection: 'column',
        position: 'relative'
      }}>
        {/* status bar */}
        <div style={{ height: 36, padding: '0 20px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, fontWeight: 600, fontVariantNumeric: 'tabular-nums' }}>
          <span>9:41</span>
          {platform === 'ios' && <div style={{ width: 96, height: 22, borderRadius: 11, background: 'var(--ink)' }} />}
          <span style={{ display: 'flex', gap: 4, alignItems: 'center', fontSize: 10 }}>●●●●● ▶ ▌</span>
        </div>
        {children}
      </div>
    </div>
  );
}

function MobOffline() {
  return (
    <MobileShell>
      <div style={{ padding: '8px 16px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ fontSize: 20, fontWeight: 700, letterSpacing: -0.3 }}>Drive</span>
        <BBChip variant="default" style={{ marginLeft: 'auto', fontSize: 10 }}>Offline</BBChip>
      </div>
      <div style={{ margin: '10px 16px', padding: 12, background: 'oklch(0.98 0.02 60)', border: '1px solid oklch(0.88 0.05 60)', borderRadius: 10, display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ width: 28, height: 28, borderRadius: '50%', background: 'oklch(0.88 0.05 60)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <Ico name="shield" size={14} color="oklch(0.45 0.08 60)" />
        </div>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>No connection</div>
          <div style={{ fontSize: 10.5, color: 'var(--ink-3)', marginTop: 1 }}>Working from your device · changes will sync when you're back</div>
        </div>
      </div>
      <div style={{ padding: '4px 16px' }}>
        <div className="t-label" style={{ marginBottom: 8, fontSize: 9.5 }}>Available offline · 12 files</div>
        {[
          { n: 'minutes-march.pdf', s: '2.4 MB', off: true },
          { n: 'interview-03.wav', s: '142 MB', off: true },
          { n: 'evidence-photos.zip', s: '89 MB', off: true },
          { n: 'court-records.pdf', s: '8.7 MB', off: false },
          { n: 'Tirana-map.png', s: '6.1 MB', off: false },
        ].map((f, i) => (
          <div key={i} style={{ padding: '10px 0', borderBottom: i < 4 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 10 }}>
            <Ico name="file" size={14} color={f.off ? 'var(--amber-deep)' : 'var(--ink-4)'} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12.5, fontWeight: 500, color: f.off ? 'var(--ink)' : 'var(--ink-3)' }}>{f.n}</div>
              <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginTop: 1 }}>{f.s}</div>
            </div>
            {f.off ? <span style={{ width: 14, height: 14, borderRadius: '50%', background: 'var(--amber-deep)', color: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 8 }}>✓</span> : <span style={{ fontSize: 16, color: 'var(--ink-4)' }}>◌</span>}
          </div>
        ))}
      </div>
      <div style={{ marginTop: 'auto', padding: 12, borderTop: '1px solid var(--line)', display: 'flex', gap: 6, background: 'var(--paper-2)' }}>
        {['Drive', 'Photos', 'Shared', 'Me'].map((t, i) => (
          <div key={t} style={{ flex: 1, textAlign: 'center', padding: '4px 0', fontSize: 10, color: i === 0 ? 'var(--amber-deep)' : 'var(--ink-3)', fontWeight: i === 0 ? 600 : 400 }}>
            <Ico name={['folder', 'image', 'users', 'shield'][i]} size={14} color={i === 0 ? 'var(--amber-deep)' : 'var(--ink-3)'} />
            <div style={{ marginTop: 2 }}>{t}</div>
          </div>
        ))}
      </div>
    </MobileShell>
  );
}

function MobUpgrade() {
  return (
    <MobileShell>
      <div style={{ padding: '8px 20px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ fontSize: 18, color: 'var(--ink-3)' }}>×</span>
        <span style={{ fontSize: 15, fontWeight: 600, margin: '0 auto' }}>Upgrade</span>
        <span style={{ width: 18 }} />
      </div>
      <div style={{ padding: '24px 20px 16px', textAlign: 'center' }}>
        <div style={{ width: 58, height: 58, margin: '0 auto', borderRadius: 14, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <BBHex size={20} />
        </div>
        <div style={{ fontSize: 20, fontWeight: 600, marginTop: 14, letterSpacing: -0.2 }}>2 TB for €4.99/mo</div>
        <div style={{ fontSize: 12, color: 'var(--ink-2)', marginTop: 6, lineHeight: 1.5 }}>You're at 18.2 of 20 GB — free plan almost full</div>
      </div>
      <div style={{ padding: '0 16px' }}>
        {[
          { l: '2 TB encrypted storage', sub: '100× more space' },
          { l: 'Unlimited version history', sub: '30 days on Free' },
          { l: 'File requests & password links', sub: 'Pro only' },
          { l: 'Priority in EU datacenter', sub: '99.95% SLA' },
        ].map(r => (
          <div key={r.l} style={{ padding: '11px 0', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ width: 18, height: 18, borderRadius: '50%', background: 'var(--amber-bg)', color: 'var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, fontWeight: 700 }}>✓</span>
            <div>
              <div style={{ fontSize: 12.5, fontWeight: 500 }}>{r.l}</div>
              <div style={{ fontSize: 10.5, color: 'var(--ink-3)', marginTop: 1 }}>{r.sub}</div>
            </div>
          </div>
        ))}
      </div>
      <div style={{ padding: 16, marginTop: 'auto' }}>
        <button style={{ width: '100%', padding: 14, borderRadius: 12, background: 'var(--amber-deep)', color: 'var(--paper)', border: 'none', fontSize: 14, fontWeight: 600 }}>Subscribe · €4.99/mo</button>
        <div style={{ fontSize: 10, color: 'var(--ink-3)', textAlign: 'center', marginTop: 10, lineHeight: 1.5 }}>Billed via App Store · cancel anytime · 7-day refund</div>
      </div>
    </MobileShell>
  );
}

function MobShareExt() {
  return (
    <div style={{ position: 'relative' }}>
      <MobileShell>
        {/* dimmed photos app behind */}
        <div style={{ flex: 1, background: 'oklch(0.14 0.005 70)', opacity: 0.6, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 1, width: '100%', height: '100%' }}>
            {Array.from({length: 12}).map((_, i) => (
              <div key={i} style={{ background: `oklch(${0.3 + (i % 4) * 0.08} 0.04 ${30 + i * 20})` }} />
            ))}
          </div>
        </div>
        {/* share sheet */}
        <div style={{ position: 'absolute', left: 0, right: 0, bottom: 0, background: 'var(--paper)', borderTopLeftRadius: 14, borderTopRightRadius: 14, padding: '14px 14px 20px', boxShadow: '0 -20px 40px -10px rgba(0,0,0,0.2)' }}>
          <div style={{ width: 36, height: 4, borderRadius: 2, background: 'var(--line-2)', margin: '0 auto 14px' }} />
          <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4 }}>3 photos selected</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 12 }}>12.4 MB · will be encrypted before upload</div>
          <div style={{ padding: 12, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 10, display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10 }}>
            <div style={{ width: 34, height: 34, borderRadius: 8, background: 'var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <BBHex size={14} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12.5, fontWeight: 600 }}>Save to Beebeeb</div>
              <div style={{ fontSize: 10.5, color: 'var(--ink-3)', marginTop: 1 }}>Tirana-story / source</div>
            </div>
            <Ico name="chevron-right" size={14} color="var(--ink-3)" />
          </div>
          <div style={{ display: 'flex', gap: 6, overflowX: 'auto' }}>
            {['Messages', 'Mail', 'AirDrop', 'Signal'].map(a => (
              <div key={a} style={{ minWidth: 64, padding: '10px 6px', background: 'var(--paper-2)', borderRadius: 10, textAlign: 'center', fontSize: 10 }}>
                <div style={{ width: 34, height: 34, borderRadius: 8, background: 'var(--line-2)', margin: '0 auto 6px' }} />
                {a}
              </div>
            ))}
          </div>
        </div>
      </MobileShell>
    </div>
  );
}

function MobPush() {
  return (
    <MobileShell>
      {/* lockscreen-ish */}
      <div style={{ flex: 1, background: 'linear-gradient(180deg, oklch(0.3 0.05 270), oklch(0.15 0.03 260))', padding: '20px 14px', color: '#fff' }}>
        <div style={{ textAlign: 'center', fontSize: 48, fontWeight: 300, letterSpacing: -1, marginTop: 24 }}>9:41</div>
        <div style={{ textAlign: 'center', fontSize: 14, opacity: 0.9 }}>Thursday, 23 April</div>
        <div style={{ marginTop: 24, display: 'flex', flexDirection: 'column', gap: 10 }}>
          {[
            { app: 'Beebeeb', t: 'Marc shared Legal-review', b: 'Keys exchanged · tap to open', time: 'now' },
            { app: 'Beebeeb', t: 'File request received', b: 'Anna sent you 2 files · encrypted to your key', time: '2m ago' },
            { app: 'Beebeeb', t: 'New sign-in from Berlin', b: 'MacBook Pro · Safari · tap if that wasn\'t you', time: '8m ago' },
          ].map((n, i) => (
            <div key={i} style={{ padding: 12, background: 'rgba(255,255,255,0.12)', backdropFilter: 'blur(20px)', borderRadius: 14, color: '#fff' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
                <div style={{ width: 18, height: 18, borderRadius: 4, background: 'var(--amber)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <BBHex size={10} />
                </div>
                <span style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: 0.4, opacity: 0.85 }}>{n.app}</span>
                <span style={{ marginLeft: 'auto', fontSize: 10, opacity: 0.7 }}>{n.time}</span>
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, lineHeight: 1.3 }}>{n.t}</div>
              <div style={{ fontSize: 11.5, opacity: 0.85, marginTop: 3, lineHeight: 1.4 }}>{n.b}</div>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 'auto', textAlign: 'center', fontSize: 10, opacity: 0.6, padding: 20 }}>
          <div>Swipe up to unlock · FaceID</div>
        </div>
      </div>
    </MobileShell>
  );
}

Object.assign(window, { MobOffline, MobUpgrade, MobShareExt, MobPush });
