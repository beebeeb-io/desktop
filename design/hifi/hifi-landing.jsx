// hifi-landing.jsx — Public landing page

function LandingNav() {
  return (
    <div style={{
      padding: '16px 32px', display: 'flex', alignItems: 'center', gap: 24,
      borderBottom: '1px solid var(--line)', background: 'var(--paper)',
    }}>
      <BBLogo size={15} />
      <div style={{ display: 'flex', gap: 20, marginLeft: 20 }}>
        {['Product', 'For teams', 'Security', 'Pricing', 'Open source'].map(l => (
          <span key={l} style={{ fontSize: 13, color: 'var(--ink-2)', cursor: 'default' }}>{l}</span>
        ))}
      </div>
      <div style={{ marginLeft: 'auto', display: 'flex', gap: 8, alignItems: 'center' }}>
        <BBRegionBadge region="EU sovereign" />
        <BBBtn size="sm" variant="ghost">Sign in</BBBtn>
        <BBBtn size="sm" variant="amber">Start free</BBBtn>
      </div>
    </div>
  );
}

function LandingHero() {
  return (
    <div style={{ padding: '72px 32px 48px', textAlign: 'center', position: 'relative', overflow: 'hidden' }}>
      {/* subtle honeycomb backdrop */}
      <div className="bb-honeycomb" style={{ position: 'absolute', inset: 0, opacity: 0.35 }} />
      <div style={{
        position: 'absolute', top: 40, left: '50%', transform: 'translateX(-50%)',
        width: 600, height: 300, borderRadius: '50%',
        background: 'radial-gradient(circle, oklch(0.92 0.1 85 / 0.45), transparent 70%)',
        filter: 'blur(40px)',
      }} />

      <div style={{ position: 'relative', maxWidth: 880, margin: '0 auto' }}>
        <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 24 }}>
          <div style={{
            display: 'inline-flex', alignItems: 'center', gap: 8,
            padding: '5px 12px 5px 5px',
            background: 'var(--paper)', border: '1px solid var(--line-2)',
            borderRadius: 999, boxShadow: 'var(--shadow-1)',
          }}>
            <span style={{ padding: '2px 8px', background: 'var(--ink)', color: 'var(--amber)', borderRadius: 999, fontFamily: 'var(--font-mono)', fontSize: 10, letterSpacing: '0.06em', textTransform: 'uppercase', fontWeight: 600 }}>
              New
            </span>
            <span style={{ fontSize: 12, color: 'var(--ink-2)' }}>
              Built for NIS2 & DORA — read our compliance note
            </span>
            <Ico name="chevRight" size={11} color="var(--ink-3)" />
          </div>
        </div>

        <div style={{ fontSize: 64, lineHeight: 1.02, letterSpacing: '-0.035em', fontWeight: 700, marginBottom: 20 }}>
          Storage that can't be read<br />
          <span style={{ color: 'var(--amber-deep)' }}>— not even by us.</span>
        </div>
        <div style={{ fontSize: 17, color: 'var(--ink-2)', maxWidth: 620, margin: '0 auto 28px', lineHeight: 1.55 }}>
          Beebeeb is the European cloud for people who want their files encrypted before they leave their device, stored on EU soil, and governed by EU law — nothing more, nothing less.
        </div>

        <div style={{ display: 'flex', gap: 10, justifyContent: 'center', marginBottom: 18 }}>
          <BBBtn variant="amber" size="lg" style={{ padding: '12px 20px' }}>
            Start free · 10 GB <Ico name="chevRight" size={13} />
          </BBBtn>
          <BBBtn size="lg" style={{ padding: '12px 20px' }} icon={<Ico name="download" size={13} />}>
            Download desktop app
          </BBBtn>
        </div>

        <div style={{ fontSize: 12, color: 'var(--ink-3)', display: 'flex', gap: 14, justifyContent: 'center' }}>
          <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <Ico name="check" size={11} color="oklch(0.55 0.12 155)" /> No card
          </span>
          <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <Ico name="check" size={11} color="oklch(0.55 0.12 155)" /> Open-source clients
          </span>
          <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <Ico name="check" size={11} color="oklch(0.55 0.12 155)" /> Audit our code on GitHub
          </span>
        </div>
      </div>

      {/* Product preview — composited file row */}
      <div style={{
        position: 'relative', maxWidth: 1040, margin: '56px auto 0',
        borderRadius: 16, background: 'var(--paper)',
        border: '1px solid var(--line-2)',
        boxShadow: '0 40px 80px -30px rgba(24,20,10,0.25), 0 10px 30px -10px rgba(24,20,10,0.1)',
        overflow: 'hidden',
      }}>
        {/* Window chrome */}
        <div style={{ padding: '10px 14px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 6 }}>
          <span style={{ width: 10, height: 10, borderRadius: 999, background: '#ff5f57' }} />
          <span style={{ width: 10, height: 10, borderRadius: 999, background: '#febc2e' }} />
          <span style={{ width: 10, height: 10, borderRadius: 999, background: '#28c840' }} />
          <div className="t-mono" style={{ marginLeft: 14, fontSize: 11, color: 'var(--ink-3)' }}>beebeeb.io / drive</div>
        </div>
        {/* Fake drive view */}
        <div style={{ padding: 0, fontFamily: 'var(--font-sans)' }}>
          <div style={{
            padding: '10px 20px', borderBottom: '1px solid var(--line)', background: 'var(--paper)',
            display: 'grid', gridTemplateColumns: '24px 1fr 110px 110px 100px', gap: 14,
          }}>
            <span />
            <span className="t-label">Name</span>
            <span className="t-label">Size</span>
            <span className="t-label">Modified</span>
            <span className="t-label">Shared</span>
          </div>
          {[
            ['folder', 'Contracts', '—', '2d ago', 3],
            ['folder', 'Q2 Financials', '—', '4h ago', 2],
            ['file', 'board-deck-apr.pdf', '4.2 MB', '12m ago', 4],
            ['file', 'term-sheet-v3.docx', '88 KB', '1h ago', 2],
          ].map((r, i) => (
            <div key={i} style={{
              padding: '11px 20px', borderBottom: '1px solid var(--line)',
              display: 'grid', gridTemplateColumns: '24px 1fr 110px 110px 100px', gap: 14, alignItems: 'center',
            }}>
              <FileIconHi type={r[0]} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 500 }}>{r[1]}</span>
                {i === 2 && <BBChip variant="amber" style={{ fontSize: 9.5 }}><Ico name="lock" size={9} /> E2EE</BBChip>}
              </div>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r[2]}</span>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r[3]}</span>
              <AvatarStack n={r[4]} />
            </div>
          ))}
        </div>

        {/* Floating encryption pill */}
        <div style={{
          position: 'absolute', bottom: 20, right: 24,
          padding: '8px 12px', borderRadius: 999,
          background: 'var(--ink)', color: 'var(--amber)',
          fontFamily: 'var(--font-mono)', fontSize: 11,
          boxShadow: '0 10px 24px -8px rgba(0,0,0,0.3)',
          display: 'flex', alignItems: 'center', gap: 6,
        }}>
          <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--amber)', boxShadow: '0 0 8px var(--amber)' }} />
          encrypting · AES-256-GCM · 14 MB/s
        </div>
      </div>
    </div>
  );
}

function LandingTrust() {
  return (
    <div style={{
      padding: '28px 32px', borderTop: '1px solid var(--line)',
      borderBottom: '1px solid var(--line)', background: 'var(--paper-2)',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 28, flexWrap: 'wrap', justifyContent: 'center', maxWidth: 1100, margin: '0 auto' }}>
        <span className="t-label" style={{ color: 'var(--ink-3)' }}>Built on EU infrastructure</span>
        {['Hetzner · Germany', 'Leaseweb · Netherlands'].map((l, i) => (
          <span key={l} style={{
            fontFamily: 'var(--font-mono)', fontSize: 12, color: 'var(--ink-2)', fontWeight: 500,
          }}>{l}</span>
        ))}
        <span style={{ color: 'var(--line-2)' }}>·</span>
        <span style={{ display: 'flex', gap: 10 }}>
          {['GDPR', 'NIS2', 'DORA', 'ISO 27001 — in progress'].map(c => (
            <BBChip key={c} style={{ fontSize: 10 }}>{c}</BBChip>
          ))}
        </span>
      </div>
    </div>
  );
}

function LandingPillars() {
  const pillars = [
    {
      ico: 'shield',
      n: '01',
      t: 'Zero-knowledge, architecturally.',
      d: 'Files are encrypted on your device with a key we never see. Our servers store opaque blobs — if subpoenaed, we hand over encrypted garbage, and that\'s the point.',
      stat: 'AES-256-GCM · Argon2id · X25519',
    },
    {
      ico: 'cloud',
      n: '02',
      t: 'European soil, European law.',
      d: 'Every byte lives in an EU-operated data centre. No CLOUD Act exposure, no US subprocessors, no fine print that moves your data to Oregon overnight.',
      stat: 'Choice of region · DE · NL · FR',
    },
    {
      ico: 'users',
      n: '03',
      t: 'Built for small teams that act big.',
      d: 'Shared vaults, team keys, client portals, audit logs, SSO. Everything regulated industries need — without the enterprise ceremony or pricing.',
      stat: 'From 5 seats · €9.99 / seat',
    },
    {
      ico: 'key',
      n: '04',
      t: 'Open where it counts.',
      d: 'Every client app — web, iOS, Android, desktop — is open source and reproducibly built. Read the code, compile it yourself, run it offline. We earn trust, we don\'t ask for it.',
      stat: 'AGPL-3.0 · audited by Cure53',
    },
  ];
  return (
    <div style={{ padding: '80px 32px', background: 'var(--paper)' }}>
      <div style={{ maxWidth: 1100, margin: '0 auto' }}>
        <div className="t-label" style={{ textAlign: 'center', marginBottom: 10 }}>Why Beebeeb</div>
        <div className="t-display" style={{ textAlign: 'center', marginBottom: 56, fontSize: 40 }}>
          Four promises. No fine print.
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 18 }}>
          {pillars.map((p, i) => (
            <div key={i} style={{
              padding: 28, border: '1px solid var(--line)',
              borderRadius: 'var(--r-3)', background: 'var(--paper)',
              position: 'relative', overflow: 'hidden',
            }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16 }}>
                <div style={{
                  width: 40, height: 40, borderRadius: 10,
                  background: 'var(--amber-bg)', color: 'var(--amber-deep)',
                  border: '1px solid oklch(0.86 0.07 90)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                }}>
                  <Ico name={p.ico} size={18} />
                </div>
                <div style={{ flex: 1 }}>
                  <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)', letterSpacing: '0.08em', marginBottom: 4 }}>{p.n}</div>
                  <div style={{ fontSize: 20, fontWeight: 700, letterSpacing: '-0.02em', marginBottom: 8, lineHeight: 1.2 }}>{p.t}</div>
                  <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.6, marginBottom: 14 }}>{p.d}</div>
                  <div style={{
                    display: 'inline-flex', padding: '4px 9px',
                    background: 'var(--paper-2)', border: '1px solid var(--line)',
                    borderRadius: 6, fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--ink-3)',
                  }}>{p.stat}</div>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function LandingCompare() {
  const rows = [
    ['End-to-end encryption by default', 'y', 'n', 'y', 'y'],
    ['Zero-knowledge keys', 'y', 'n', 'y', 'y'],
    ['EU-only infrastructure', 'y', 'opt', 'y', 'opt'],
    ['Not subject to US CLOUD Act', 'y', 'n', 'y', 'n'],
    ['Open-source client apps', 'y', 'n', 'partial', 'n'],
    ['SSO + audit logs on Team tier', 'y', 'addon', 'addon', 'y'],
    ['Honest "we can\'t recover your data" UX', 'y', 'n', 'partial', 'partial'],
  ];
  const Cell = ({ v }) => {
    if (v === 'y') return <div style={{ width: 20, height: 20, borderRadius: 999, background: 'oklch(0.94 0.06 155)', color: 'oklch(0.35 0.12 155)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', margin: '0 auto' }}><Ico name="check" size={11} /></div>;
    if (v === 'n') return <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-4)' }}>—</span>;
    return <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', background: 'var(--paper-2)', padding: '2px 6px', borderRadius: 4 }}>{v}</span>;
  };
  return (
    <div style={{ padding: '72px 32px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
      <div style={{ maxWidth: 1000, margin: '0 auto' }}>
        <div className="t-label" style={{ textAlign: 'center', marginBottom: 10 }}>The honest comparison</div>
        <div className="t-display" style={{ textAlign: 'center', marginBottom: 36, fontSize: 40 }}>
          How we stack up — without the spin.
        </div>
        <div className="bb-card" style={{ padding: 0 }}>
          <div style={{
            display: 'grid', gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr', gap: 14,
            padding: '14px 22px', background: 'var(--paper)',
            borderBottom: '1px solid var(--line)',
          }}>
            <span />
            <div style={{ textAlign: 'center', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2 }}>
              <BBLogo size={14} word={false} />
              <span style={{ fontSize: 12, fontWeight: 700 }}>Beebeeb</span>
            </div>
            {['Dropbox', 'Proton', 'Tresorit'].map(n => (
              <div key={n} style={{ textAlign: 'center', fontSize: 12, fontWeight: 500, color: 'var(--ink-3)' }}>{n}</div>
            ))}
          </div>
          {rows.map((r, i) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr', gap: 14,
              padding: '13px 22px', borderBottom: i < rows.length - 1 ? '1px solid var(--line)' : 'none',
              alignItems: 'center',
            }}>
              <span style={{ fontSize: 13, color: 'var(--ink-2)' }}>{r[0]}</span>
              {[1, 2, 3, 4].map(j => (
                <div key={j} style={{ textAlign: 'center' }}><Cell v={r[j]} /></div>
              ))}
            </div>
          ))}
        </div>
        <div className="t-micro" style={{ textAlign: 'center', color: 'var(--ink-4)', marginTop: 14, fontStyle: 'italic' }}>
          Comparison as of April 2026 based on public documentation. We keep this table current — including when competitors catch up.
        </div>
      </div>
    </div>
  );
}

function LandingBrother() {
  return (
    <div style={{ padding: '72px 32px', background: 'var(--paper)' }}>
      <div style={{ maxWidth: 780, margin: '0 auto', textAlign: 'center' }}>
        <div className="t-label" style={{ marginBottom: 10 }}>A note from the founders</div>
        <div style={{ fontSize: 22, lineHeight: 1.5, color: 'var(--ink-2)', letterSpacing: '-0.015em', fontStyle: 'italic', marginBottom: 28 }}>
          "We're two brothers from Wijchen who spent years building reliable systems at other companies. Every European SMB we know is one CLOUD Act away from an uncomfortable conversation with their customers. We wanted one answer: encrypted on your device, stored in the EU, governed by EU law. That's the whole product."
        </div>
        <div style={{ display: 'flex', gap: 16, justifyContent: 'center', alignItems: 'center' }}>
          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            <div style={{ width: 36, height: 36, borderRadius: 999, background: 'var(--amber)', fontWeight: 700, fontSize: 12, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>ID</div>
            <div style={{ textAlign: 'left' }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>Bram & Guus Langelaar</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)' }}>CEO & CTO · Wijchen, NL</div>
            </div>
          </div>
          <div style={{ width: 1, height: 24, background: 'var(--line-2)' }} />
          <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>beebeeb.io</div>
        </div>
      </div>
    </div>
  );
}

function LandingCTA() {
  return (
    <div style={{
      padding: '72px 32px',
      background: 'var(--ink)',
      color: 'var(--paper)',
      position: 'relative', overflow: 'hidden',
    }}>
      <div className="bb-honeycomb" style={{
        position: 'absolute', inset: 0, opacity: 0.08,
        backgroundImage: 'radial-gradient(circle at 50% 50%, oklch(0.82 0.17 84 / 0.5) 1.5px, transparent 1.8px)',
      }} />
      <div style={{ position: 'relative', maxWidth: 780, margin: '0 auto', textAlign: 'center' }}>
        <div style={{ fontSize: 44, lineHeight: 1.05, letterSpacing: '-0.03em', fontWeight: 700, marginBottom: 16 }}>
          Start encrypting<br />
          <span style={{ color: 'var(--amber)' }}>in the next 90 seconds.</span>
        </div>
        <div style={{ fontSize: 15, color: 'oklch(0.78 0.01 80)', marginBottom: 28, maxWidth: 520, margin: '0 auto 28px', lineHeight: 1.6 }}>
          10 GB free forever. No card, no upsell email storm, no dark patterns on the way out.
        </div>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'center' }}>
          <BBBtn variant="amber" size="lg" style={{ padding: '12px 22px' }}>
            Create your account <Ico name="chevRight" size={13} />
          </BBBtn>
          <BBBtn size="lg" style={{
            padding: '12px 22px', background: 'transparent',
            color: 'var(--paper)', borderColor: 'oklch(0.35 0.01 70)',
          }}>
            Read the whitepaper
          </BBBtn>
        </div>
      </div>
    </div>
  );
}

function LandingFooter() {
  const cols = [
    ['Product', ['Drive', 'Photos', 'Sharing', 'Team vaults', 'Desktop apps', 'Mobile apps']],
    ['Security', ['Whitepaper', 'Open source', 'Audit reports', 'Bug bounty', 'Security page']],
    ['Company', ['About', 'Careers', 'Blog', 'Press', 'Contact']],
    ['Legal', ['Privacy', 'Terms', 'DPA', 'Transparency report', 'Imprint']],
  ];
  return (
    <div style={{ padding: '48px 32px 28px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
      <div style={{ maxWidth: 1100, margin: '0 auto' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1.4fr repeat(4, 1fr)', gap: 24, marginBottom: 36 }}>
          <div>
            <BBLogo size={15} />
            <div style={{ fontSize: 12, color: 'var(--ink-3)', marginTop: 12, lineHeight: 1.6, maxWidth: 260 }}>
              Beebeeb · operated by Initlabs B.V.<br />
              Kelvinstraat 34A, 6601 HE Wijchen<br />
              KvK 95157565 · BTW NL867023430B01
            </div>
          </div>
          {cols.map(([title, items], i) => (
            <div key={i}>
              <div className="t-label" style={{ marginBottom: 10 }}>{title}</div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {items.map(l => (
                  <span key={l} style={{ fontSize: 12.5, color: 'var(--ink-2)' }}>{l}</span>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div style={{
          paddingTop: 20, borderTop: '1px solid var(--line)',
          display: 'flex', alignItems: 'center', gap: 14, fontSize: 11, color: 'var(--ink-3)', flexWrap: 'wrap',
        }}>
          <span>© 2026 Initlabs B.V.</span>
          <span>·</span>
          <BBRegionBadge />
          <span style={{ marginLeft: 'auto', display: 'flex', gap: 10, alignItems: 'center' }}>
            <span>Status</span>
            <span className="bb-chip green"><span className="dot" /> All systems</span>
          </span>
        </div>
      </div>
    </div>
  );
}

function HiLanding() {
  return (
    <div style={{
      width: 1280, background: 'var(--paper)',
      border: '1px solid var(--line-2)', borderRadius: 'var(--r-3)',
      overflow: 'hidden', boxShadow: 'var(--shadow-3)',
    }}>
      <LandingNav />
      <LandingHero />
      <LandingTrust />
      <LandingPillars />
      <LandingCompare />
      <LandingBrother />
      <LandingCTA />
      <LandingFooter />
    </div>
  );
}

Object.assign(window, { HiLanding });
