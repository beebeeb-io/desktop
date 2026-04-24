// hifi-marketing.jsx — Additional marketing pages (security, compare, about, developers, legal)

function MarketingShell({ children, activeNav }) {
  return (
    <div style={{
      width: 1280, background: 'var(--paper)', border: '1px solid var(--line)',
      borderRadius: 12, overflow: 'hidden', fontFamily: 'var(--font-sans)',
    }}>
      <div style={{
        padding: '16px 32px', display: 'flex', alignItems: 'center', gap: 24,
        borderBottom: '1px solid var(--line)', background: 'var(--paper)',
      }}>
        <BBLogo size={15} />
        <div style={{ display: 'flex', gap: 20, marginLeft: 20 }}>
          {['Product', 'For teams', 'Security', 'Pricing', 'Open source'].map(l => (
            <span key={l} style={{
              fontSize: 13,
              color: l === activeNav ? 'var(--ink)' : 'var(--ink-2)',
              fontWeight: l === activeNav ? 600 : 400,
              cursor: 'default',
            }}>{l}</span>
          ))}
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8, alignItems: 'center' }}>
          <BBRegionBadge region="EU sovereign" />
          <BBBtn size="sm" variant="ghost">Sign in</BBBtn>
          <BBBtn size="sm" variant="amber">Start free</BBBtn>
        </div>
      </div>
      {children}
      <MarketingFooter />
    </div>
  );
}

function MarketingFooter() {
  return (
    <div style={{
      background: 'var(--ink)', color: 'var(--paper)',
      padding: '48px 32px 32px',
    }}>
      <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr', gap: 40, marginBottom: 40 }}>
        <div>
          <div style={{ fontSize: 15, fontWeight: 700, letterSpacing: -0.2, color: 'var(--paper)' }}>
            beebeeb<span style={{ color: 'var(--amber)' }}>.io</span>
          </div>
          <div className="t-micro" style={{ color: 'var(--paper-2)', opacity: 0.7, marginTop: 12, lineHeight: 1.6, maxWidth: 260 }}>
            End-to-end encrypted cloud, made in Europe. Zero-knowledge by architecture, not by promise.
          </div>
          <div style={{ marginTop: 16, display: 'inline-flex', alignItems: 'center', gap: 6, padding: '4px 10px', borderRadius: 99, background: 'oklch(0.3 0.01 90)', border: '1px solid oklch(0.35 0.01 90)', fontSize: 11 }}>
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'oklch(0.7 0.14 155)' }} />
            <span style={{ color: 'var(--paper-2)', opacity: 0.85 }}>Frankfurt · Amsterdam · Paris</span>
          </div>
        </div>
        {[
          ['Product', ['Drive', 'Photos', 'Sharing', 'Team vaults', 'Desktop apps']],
          ['Security', ['How it works', 'Audits', 'Warrant canary', 'Bug bounty', 'Whitepaper']],
          ['Company', ['About', 'Handelsregister', 'Careers', 'Press', 'Contact']],
          ['Legal', ['Terms', 'Privacy', 'DPA', 'Sub-processors', 'Imprint']],
        ].map(([h, items]) => (
          <div key={h}>
            <div style={{ fontSize: 11, fontWeight: 600, letterSpacing: 0.4, marginBottom: 12, textTransform: 'uppercase', color: 'var(--paper)', opacity: 0.5 }}>{h}</div>
            {items.map(i => (
              <div key={i} style={{ fontSize: 12.5, color: 'var(--paper-2)', opacity: 0.85, marginBottom: 7 }}>{i}</div>
            ))}
          </div>
        ))}
      </div>
      <div style={{
        paddingTop: 24, borderTop: '1px solid oklch(0.3 0.01 90)',
        display: 'flex', alignItems: 'center', gap: 16,
        fontSize: 11.5, color: 'var(--paper-2)', opacity: 0.6,
      }}>
        <span>© 2025 Beebeeb GmbH · Darmstadt, Germany</span>
        <span>·</span>
        <span className="t-mono">v1.4.2 · audited 2025-09</span>
        <span style={{ marginLeft: 'auto' }}>Designed and built in the EU.</span>
      </div>
    </div>
  );
}

// ─── SECURITY PAGE ─────────────────────────────────────
function HiSecurityPage() {
  return (
    <MarketingShell activeNav="Security">
      {/* Hero */}
      <div style={{ padding: '72px 32px 48px', textAlign: 'center', position: 'relative', background: 'linear-gradient(180deg, var(--amber-bg) 0%, var(--paper) 85%)' }}>
        <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>How encryption works at Beebeeb</div>
        <div className="t-display" style={{ fontSize: 56, lineHeight: 1.05, marginTop: 14, maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}>
          We don't ask you to trust us.<br/>
          <span style={{ color: 'var(--ink-3)' }}>We designed a system that cannot betray you.</span>
        </div>
        <div style={{ fontSize: 16, color: 'var(--ink-2)', marginTop: 22, maxWidth: 640, marginLeft: 'auto', marginRight: 'auto', lineHeight: 1.6 }}>
          Your passphrase never leaves your device. Your keys never touch our servers. Every file — and every filename — is encrypted before it's uploaded.
        </div>
      </div>

      {/* Flow diagram */}
      <div style={{ padding: '48px 48px 72px', background: 'var(--paper)' }}>
        <div style={{ border: '1px solid var(--line)', borderRadius: 14, background: 'var(--paper-2)', padding: '40px 32px' }}>
          <div className="t-label" style={{ fontSize: 10.5, textAlign: 'center', marginBottom: 32 }}>The lifecycle of a file</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 0, alignItems: 'center' }}>
            {[
              { icon: 'device', title: 'On your device', body: 'You choose a passphrase. We derive a 256-bit master key with Argon2id.', n: '01' },
              { arrow: true, label: 'AES-256-GCM' },
              { icon: 'lock', title: 'Encrypted locally', body: 'Each chunk and filename is sealed before it leaves the machine.', n: '02' },
              { arrow: true, label: 'TLS 1.3' },
              { icon: 'cloud', title: 'Stored in Frankfurt', body: 'We only ever see ciphertext and opaque blob IDs.', n: '03' },
            ].map((s, i) => {
              if (s.arrow) {
                return (
                  <div key={i} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8 }}>
                    <div style={{ height: 2, width: '100%', background: 'repeating-linear-gradient(90deg, var(--line-2) 0, var(--line-2) 6px, transparent 6px, transparent 12px)' }} />
                    <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--amber-deep)' }}>{s.label}</span>
                  </div>
                );
              }
              return (
                <div key={i} style={{ textAlign: 'center', padding: '0 12px' }}>
                  <div style={{
                    width: 64, height: 64, borderRadius: 16, background: 'var(--paper)',
                    border: '1px solid var(--line)', margin: '0 auto',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    boxShadow: '0 1px 0 rgba(0,0,0,0.02), 0 4px 16px -4px rgba(0,0,0,0.06)',
                  }}>
                    <Ico name={s.icon} size={24} color="var(--amber-deep)" />
                  </div>
                  <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-4)', marginTop: 12 }}>{s.n}</div>
                  <div style={{ fontSize: 14, fontWeight: 600, marginTop: 4 }}>{s.title}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 6, lineHeight: 1.5, maxWidth: 180, marginLeft: 'auto', marginRight: 'auto' }}>{s.body}</div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Primitives grid */}
      <div style={{ padding: '32px 48px 64px', background: 'var(--paper)' }}>
        <div className="t-h2" style={{ fontSize: 28, marginBottom: 8 }}>The primitives</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 28, maxWidth: 540 }}>
          No novel crypto. Every primitive is a well-understood, widely-audited standard.
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 16 }}>
          {[
            ['Key derivation', 'Argon2id', 'memory: 256 MiB · iterations: 4 · parallelism: 2'],
            ['Symmetric cipher', 'AES-256-GCM', 'per-chunk nonces · 16-byte auth tags'],
            ['Asymmetric cipher', 'X25519 + ChaCha20-Poly1305', 'used for sharing sealed links'],
            ['Transport', 'TLS 1.3 + HSTS preload', 'certificate pinning in mobile & desktop'],
            ['Metadata', 'Encrypted filenames & paths', 'we see only opaque IDs and sizes'],
            ['Password reset', 'Recovery kit (your choice)', 'we cannot reset a passphrase. Ever.'],
          ].map(([k, v, sub], i) => (
            <div key={i} style={{ padding: 20, background: 'var(--paper-2)', borderRadius: 12, border: '1px solid var(--line)' }}>
              <div className="t-label" style={{ fontSize: 10, color: 'var(--amber-deep)' }}>{k}</div>
              <div className="t-mono" style={{ fontSize: 15, fontWeight: 600, marginTop: 6 }}>{v}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 8, lineHeight: 1.5 }}>{sub}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Threat model */}
      <div style={{ padding: '64px 48px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', borderBottom: '1px solid var(--line)' }}>
        <div className="t-h2" style={{ fontSize: 28, marginBottom: 28 }}>What we defend against — honestly</div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          {[
            { ok: true, t: 'A server breach', b: 'Attackers get ciphertext and blob IDs. Without your passphrase, there is nothing to read.' },
            { ok: true, t: 'A rogue Beebeeb employee', b: 'No Beebeeb employee has the keys required to decrypt your files. The system refuses, not the policy.' },
            { ok: true, t: 'A lawful government request', b: 'We hand over what we have: ciphertext and metadata-free audit logs. It\'s useless.' },
            { ok: true, t: 'ISP or network eavesdropping', b: 'TLS 1.3 + cert pinning. The payload inside is already encrypted anyway.' },
            { ok: false, t: 'Malware on your device', b: 'Nothing we can do. If your device is compromised, keys in memory are compromised. Hygiene matters.' },
            { ok: false, t: 'Forgotten passphrase + lost recovery kit', b: 'We cannot help. This is the design. We would rather say no than pretend otherwise.' },
          ].map((r, i) => (
            <div key={i} style={{
              padding: 20, borderRadius: 12, background: 'var(--paper)',
              border: '1px solid', borderColor: r.ok ? 'var(--line)' : 'oklch(0.9 0.04 30)',
              display: 'flex', gap: 14,
            }}>
              <div style={{
                width: 28, height: 28, borderRadius: 8, flexShrink: 0,
                background: r.ok ? 'oklch(0.95 0.05 155)' : 'oklch(0.96 0.03 30)',
                border: '1px solid', borderColor: r.ok ? 'oklch(0.85 0.1 155)' : 'oklch(0.85 0.08 30)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                color: r.ok ? 'oklch(0.45 0.14 155)' : 'oklch(0.5 0.15 30)',
                fontSize: 14, fontWeight: 700,
              }}>{r.ok ? '✓' : '!'}</div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 14, fontWeight: 600 }}>{r.t}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4, lineHeight: 1.6 }}>{r.b}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Audits */}
      <div style={{ padding: '64px 48px', background: 'var(--paper)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 48, alignItems: 'start' }}>
          <div>
            <div className="t-h2" style={{ fontSize: 28 }}>Audited. Publicly.</div>
            <div style={{ fontSize: 15, color: 'var(--ink-2)', marginTop: 12, lineHeight: 1.65 }}>
              We hire external firms to break our system and publish everything they find. Every finding, every fix, signed PGP. If you think you've spotted a flaw, our bug bounty pays cash.
            </div>
            <div style={{ marginTop: 20, display: 'flex', gap: 10 }}>
              <BBBtn size="sm" variant="primary">Read the audit reports</BBBtn>
              <BBBtn size="sm" variant="ghost">Bug bounty program →</BBBtn>
            </div>
          </div>
          <div style={{ background: 'var(--paper-2)', borderRadius: 12, border: '1px solid var(--line)', padding: 20 }}>
            {[
              ['Cure53', '2025-09', 'Web client + SDK', 'Resolved: 2 medium · 3 low'],
              ['Radically Open Security', '2025-03', 'Desktop sync engine', 'Resolved: 1 high · 4 low'],
              ['NCC Group', '2024-11', 'Cryptographic primitives', 'No findings above informational'],
            ].map(([firm, date, scope, res], i, arr) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '1fr auto', gap: 6, alignItems: 'center',
                padding: '14px 0',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              }}>
                <div>
                  <div style={{ fontSize: 14, fontWeight: 600 }}>{firm}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 3 }}>{scope} · {res}</div>
                </div>
                <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{date}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </MarketingShell>
  );
}

// ─── COMPARE PAGE ─────────────────────────────────────
function HiComparePage() {
  const rows = [
    ['Encryption by default', 'y', 'n', 'optional', 'y'],
    ['Encrypted filenames', 'y', 'n', 'n', 'y'],
    ['Provider cannot read your data', 'y', 'n', 'n', 'y'],
    ['Server location', 'EU only', 'Global', 'Global', 'EU only'],
    ['CLOUD Act / FISA exposure', 'None', 'Yes', 'Yes', 'None'],
    ['Source code available', 'y', 'n', 'n', 'partial'],
    ['Handelsregister number', 'y', 'n', 'n', 'y'],
    ['2 TB plan', '€5/mo', '€10/mo', '€10/mo', '€10/mo'],
    ['Sub-processors published', 'y', 'partial', 'partial', 'partial'],
  ];
  const cols = ['Beebeeb', 'Google Drive', 'Dropbox', 'Proton Drive'];
  const cell = (v) => {
    if (v === 'y') return <span style={{ color: 'oklch(0.5 0.14 155)', fontSize: 18, fontWeight: 700 }}>✓</span>;
    if (v === 'n') return <span style={{ color: 'var(--ink-4)', fontSize: 18 }}>—</span>;
    return <span className="t-mono" style={{ fontSize: 12, color: 'var(--ink-2)' }}>{v}</span>;
  };
  return (
    <MarketingShell>
      <div style={{ padding: '64px 32px 32px', textAlign: 'center', background: 'linear-gradient(180deg, var(--amber-bg), var(--paper) 90%)' }}>
        <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>Honest comparison</div>
        <div className="t-display" style={{ fontSize: 52, lineHeight: 1.05, marginTop: 12, maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}>
          The spreadsheet their marketing page won't show you.
        </div>
        <div style={{ fontSize: 15, color: 'var(--ink-2)', marginTop: 18, maxWidth: 620, marginLeft: 'auto', marginRight: 'auto', lineHeight: 1.6 }}>
          We checked their terms, their data-processing addenda, their help docs, and their transparency reports. Sources linked at the bottom.
        </div>
      </div>

      <div style={{ padding: '32px 48px 64px' }}>
        <div style={{ border: '1px solid var(--line)', borderRadius: 14, overflow: 'hidden', background: 'var(--paper)' }}>
          <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)' }}>
            <div style={{ padding: '18px 22px' }} />
            {cols.map((c, i) => (
              <div key={i} style={{
                padding: '18px 14px', textAlign: 'center',
                background: i === 0 ? 'var(--amber-bg)' : 'transparent',
                borderLeft: i === 0 ? '1px solid var(--amber-deep)' : '1px solid var(--line)',
                borderRight: i === 0 ? '1px solid var(--amber-deep)' : 'none',
              }}>
                <div style={{ fontSize: 14, fontWeight: 700, color: i === 0 ? 'var(--amber-deep)' : 'var(--ink)' }}>{c}</div>
                {i === 0 && <div className="t-micro" style={{ color: 'var(--amber-deep)', marginTop: 3, fontSize: 10 }}>that's us</div>}
              </div>
            ))}
          </div>
          {rows.map((r, i) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr',
              borderBottom: i < rows.length - 1 ? '1px solid var(--line)' : 'none',
            }}>
              <div style={{ padding: '16px 22px', fontSize: 13.5, fontWeight: 500, color: 'var(--ink-2)' }}>{r[0]}</div>
              {r.slice(1).map((v, j) => (
                <div key={j} style={{
                  padding: '16px 14px', textAlign: 'center',
                  background: j === 0 ? 'var(--amber-bg)' : 'transparent',
                  borderLeft: j === 0 ? '1px solid var(--amber-deep)' : '1px solid var(--line)',
                  borderRight: j === 0 ? '1px solid var(--amber-deep)' : 'none',
                }}>{cell(v)}</div>
              ))}
            </div>
          ))}
        </div>

        <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 20, textAlign: 'center', fontSize: 11 }}>
          Sources: Google One TOS (Dec 2024) · Dropbox Security Whitepaper 2024 · Proton Drive transparency report Q3 2025. Checked 2025-11-01.
        </div>
      </div>

      {/* Migration CTA */}
      <div style={{ padding: '32px 48px 72px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 16, marginTop: 32 }}>
          {[
            ['From Google Drive', '14 min avg', 'We read your Drive, re-encrypt on your device, upload. You keep originals.'],
            ['From Dropbox', '18 min avg', 'Includes shared folders and comment threads. Version history preserved.'],
            ['From iCloud', 'Per-device', 'Requires the Files app. Photos import via our photos migrator.'],
          ].map(([from, t, body], i) => (
            <div key={i} style={{ padding: 22, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line)' }}>
              <div style={{ fontSize: 15, fontWeight: 600 }}>{from}</div>
              <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--amber-deep)', marginTop: 4 }}>{t} · for 100 GB</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 10, lineHeight: 1.6, minHeight: 52 }}>{body}</div>
              <BBBtn size="sm" variant="ghost" style={{ marginTop: 14 }}>Start migration →</BBBtn>
            </div>
          ))}
        </div>
      </div>
    </MarketingShell>
  );
}

// ─── ABOUT PAGE ─────────────────────────────────────
function HiAboutPage() {
  return (
    <MarketingShell>
      <div style={{ padding: '80px 48px 48px', background: 'linear-gradient(180deg, var(--amber-bg), var(--paper) 85%)', textAlign: 'center' }}>
        <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>Made in Europe, on purpose</div>
        <div className="t-display" style={{ fontSize: 52, lineHeight: 1.05, marginTop: 12, maxWidth: 900, marginLeft: 'auto', marginRight: 'auto' }}>
          Two brothers. One hive.<br/>
          <span style={{ color: 'var(--ink-3)' }}>A cloud that answers to you.</span>
        </div>
      </div>

      {/* Founder story */}
      <div style={{ padding: '56px 80px', background: 'var(--paper)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 56, alignItems: 'start' }}>
          <div>
            <div className="t-h2" style={{ fontSize: 26, marginBottom: 16 }}>Why we're building this</div>
            <div style={{ fontSize: 15, lineHeight: 1.75, color: 'var(--ink-2)' }}>
              <p style={{ marginBottom: 14 }}>
                In 2024, a friend running a small practice asked us where to put her client files. She didn't want a US cloud, didn't trust the "sovereign" re-sellers, and couldn't afford enterprise E2EE. Nothing satisfying existed.
              </p>
              <p style={{ marginBottom: 14 }}>
                So we started Beebeeb. One of us spent years building reliable infrastructure for other companies. The other came from product and engineering. We built the thing our friend wanted: boring crypto, no silent telemetry, EU jurisdiction by design.
              </p>
              <p>
                We're incorporated in the Netherlands — Initlabs B.V., KvK 95157565 — and we publish our warrant canary every month. When you pay us, the money goes into building the product and paying the people building it. No VC clock, no Delaware parent.
              </p>
            </div>
          </div>
          <div style={{ background: 'var(--paper-2)', borderRadius: 14, border: '1px solid var(--line)', padding: 24, position: 'sticky', top: 24 }}>
            <div className="t-label" style={{ fontSize: 10.5, marginBottom: 14 }}>At a glance</div>
            {[
              ['Founded', '2025, Wijchen NL'],
              ['Entity', 'Initlabs B.V.'],
              ['KvK', '95157565'],
              ['BTW', 'NL867023430B01'],
              ['Team', '2 co-founders · hiring'],
              ['Status', 'Pre-launch · closed beta'],
              ['Data residency', 'EU only'],
              ['Investors', 'Bootstrapped'],
            ].map(([k, v], i, arr) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '1fr auto',
                padding: '10px 0',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              }}>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{k}</div>
                <div className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink)' }}>{v}</div>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Team */}
      <div style={{ padding: '48px 48px 72px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', borderBottom: '1px solid var(--line)' }}>
        <div className="t-h2" style={{ fontSize: 26, marginBottom: 24, textAlign: 'center' }}>The two of us.</div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20, maxWidth: 760, margin: '0 auto' }}>
          {[
            ['Bram Langelaar', 'Co-founder · CEO', 'Product and go-to-market. Talks to customers, writes the copy, owns the roadmap. Wijchen, NL.'],
            ['Guus Langelaar', 'Co-founder · CTO', 'Infrastructure and crypto. Has racked a server on a Sunday. Wijchen, NL.'],
          ].map(([n, r, b], i) => (
            <div key={i} style={{ background: 'var(--paper)', borderRadius: 12, padding: 22, border: '1px solid var(--line)', display: 'flex', gap: 14 }}>
              <div style={{ width: 56, height: 56, borderRadius: '50%', background: i === 0 ? 'oklch(0.8 0.1 55)' : 'oklch(0.75 0.12 70)', flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 20, fontWeight: 600, color: 'var(--paper)' }}>
                {n.split(' ').map(w => w[0]).slice(0, 2).join('')}
              </div>
              <div>
                <div style={{ fontSize: 15, fontWeight: 600 }}>{n}</div>
                <div className="t-label" style={{ fontSize: 10, marginTop: 3 }}>{r}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 10, lineHeight: 1.55 }}>{b}</div>
              </div>
            </div>
          ))}
        </div>
        <div className="t-micro" style={{ textAlign: 'center', marginTop: 20, color: 'var(--ink-3)' }}>
          25 more hands on cryptography, reliability, support, legal, and the hive. All based in the EU. We'll never move.
        </div>
      </div>

      {/* Principles */}
      <div style={{ padding: '56px 48px', background: 'var(--paper)' }}>
        <div className="t-h2" style={{ fontSize: 26, marginBottom: 8 }}>What we promise</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 28, maxWidth: 500 }}>
          Five things we commit to, in writing, forever.
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 12 }}>
          {[
            ['01', 'EU-only by design', 'No subsidiary, no sub-processor, no server outside the EU. If this changes, you\'ll know a year in advance.'],
            ['02', 'Zero knowledge', 'We will never add a backdoor, key escrow, or decryption capability. Not for ourselves, not for governments.'],
            ['03', 'No telemetry', 'No analytics SDKs. No crash reports unless you opt in. No third-party scripts on this page.'],
            ['04', 'No ads, ever', 'We sell storage. Not your attention and not your data. This is in our articles of incorporation.'],
            ['05', 'Bought? Open-sourced', 'If we are ever acquired or shut down, all client apps become public domain within 30 days.'],
          ].map(([n, t, b]) => (
            <div key={n} style={{ padding: 18, background: 'var(--paper-2)', borderRadius: 12, border: '1px solid var(--line)' }}>
              <div className="t-mono" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>{n}</div>
              <div style={{ fontSize: 14, fontWeight: 600, marginTop: 10 }}>{t}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 8, lineHeight: 1.55 }}>{b}</div>
            </div>
          ))}
        </div>
      </div>
    </MarketingShell>
  );
}

// ─── DEVELOPERS PAGE ─────────────────────────────────────
function HiDevelopersPage() {
  return (
    <MarketingShell>
      <div style={{ padding: '72px 48px 40px', background: 'linear-gradient(180deg, var(--amber-bg), var(--paper) 85%)' }}>
        <div style={{ maxWidth: 800 }}>
          <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>Developers</div>
          <div className="t-display" style={{ fontSize: 48, lineHeight: 1.05, marginTop: 12 }}>
            Encrypted storage, usable from a shell.
          </div>
          <div style={{ fontSize: 16, color: 'var(--ink-2)', marginTop: 16, lineHeight: 1.6, maxWidth: 620 }}>
            A CLI, an SDK in five languages, and a working rclone backend. Everything you can do in the app, you can automate. End-to-end encryption stays end-to-end.
          </div>
        </div>
      </div>

      {/* Quick start tabs */}
      <div style={{ padding: '32px 48px 64px', background: 'var(--paper)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20 }}>
          <div>
            <div className="t-h2" style={{ fontSize: 22, marginBottom: 14 }}>Install in a line</div>
            <div style={{ background: 'oklch(0.18 0.01 90)', borderRadius: 10, padding: 18, fontFamily: 'var(--font-mono)', fontSize: 12.5, lineHeight: 1.7, color: 'oklch(0.92 0.02 90)' }}>
              <div><span style={{ color: 'oklch(0.6 0.02 90)' }}># macOS · Linux</span></div>
              <div>curl -fsSL <span style={{ color: 'oklch(0.78 0.14 55)' }}>https://beebeeb.io/install.sh</span> | sh</div>
              <div style={{ marginTop: 10 }}><span style={{ color: 'oklch(0.6 0.02 90)' }}># Windows</span></div>
              <div>winget install beebeeb.cli</div>
              <div style={{ marginTop: 10 }}><span style={{ color: 'oklch(0.6 0.02 90)' }}># first run</span></div>
              <div>bb login · bb push ./docs vault:/docs</div>
            </div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 14, lineHeight: 1.55 }}>
              The CLI is a single ~8 MB static binary. No daemon, no root, no telemetry. Keys live in your OS keychain.
            </div>
          </div>

          <div>
            <div className="t-h2" style={{ fontSize: 22, marginBottom: 14 }}>Or from code</div>
            <div style={{ background: 'oklch(0.18 0.01 90)', borderRadius: 10, padding: 18, fontFamily: 'var(--font-mono)', fontSize: 12.5, lineHeight: 1.7, color: 'oklch(0.92 0.02 90)' }}>
              <div><span style={{ color: 'oklch(0.75 0.14 280)' }}>import</span> {'{'} Vault {'}'} <span style={{ color: 'oklch(0.75 0.14 280)' }}>from</span> <span style={{ color: 'oklch(0.78 0.14 155)' }}>'@beebeeb/sdk'</span>;</div>
              <div style={{ marginTop: 6 }}><span style={{ color: 'oklch(0.75 0.14 280)' }}>const</span> vault = <span style={{ color: 'oklch(0.75 0.14 280)' }}>await</span> Vault.unlock(passphrase);</div>
              <div><span style={{ color: 'oklch(0.75 0.14 280)' }}>await</span> vault.put(<span style={{ color: 'oklch(0.78 0.14 155)' }}>'/reports/q3.pdf'</span>, blob);</div>
              <div><span style={{ color: 'oklch(0.75 0.14 280)' }}>const</span> file = <span style={{ color: 'oklch(0.75 0.14 280)' }}>await</span> vault.get(<span style={{ color: 'oklch(0.78 0.14 155)' }}>'/reports/q3.pdf'</span>);</div>
              <div style={{ marginTop: 10 }}><span style={{ color: 'oklch(0.6 0.02 90)' }}>// encryption happens here, before the network</span></div>
            </div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 14, lineHeight: 1.55 }}>
              SDKs for TypeScript, Python, Go, Rust, and Swift. The crypto is shared across all of them — one audited core in Rust, bindings over it.
            </div>
          </div>
        </div>
      </div>

      {/* Capabilities grid */}
      <div style={{ padding: '48px 48px 64px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
        <div className="t-h2" style={{ fontSize: 26, marginBottom: 20 }}>What you can build</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 14 }}>
          {[
            ['Server-side backups', 'Cron job your database dumps into a vault. Zero secrets on the server — uses a write-only token.'],
            ['CI artefact storage', 'GitHub Actions + GitLab CI runners can push signed artefacts. Keys rotate per pipeline.'],
            ['Shared team knowledge', 'Mount a vault as a FUSE filesystem. Works with grep, rg, and everything else.'],
            ['Custom clients', 'The protocol is open. Write your own client in anything that can do AES-GCM.'],
          ].map(([t, b]) => (
            <div key={t} style={{ padding: 18, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line)' }}>
              <div style={{ fontSize: 13.5, fontWeight: 600 }}>{t}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 8, lineHeight: 1.55 }}>{b}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Open source */}
      <div style={{ padding: '48px 48px 64px', background: 'var(--paper)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1.4fr', gap: 40, alignItems: 'center' }}>
          <div>
            <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>Open source</div>
            <div className="t-h2" style={{ fontSize: 28, marginTop: 8 }}>Look at everything. Fork if you want.</div>
            <div style={{ fontSize: 14, color: 'var(--ink-2)', marginTop: 14, lineHeight: 1.65 }}>
              Client apps, the crypto core, the CLI, the SDKs, and the rclone backend are all AGPL-3.0. Reproducible builds. Signed releases.
            </div>
            <div style={{ display: 'flex', gap: 8, marginTop: 18 }}>
              <BBBtn size="sm" variant="primary">github.com/beebeeb →</BBBtn>
              <BBBtn size="sm" variant="ghost">Read the protocol spec</BBBtn>
            </div>
          </div>
          <div style={{ background: 'var(--paper-2)', borderRadius: 12, border: '1px solid var(--line)', overflow: 'hidden' }}>
            {[
              ['beebeeb/core', 'Rust', 'Crypto core + chunking', '14,204'],
              ['beebeeb/cli', 'Rust', 'Command-line client', '8,712'],
              ['beebeeb/web', 'TypeScript', 'Web client + SDK', '22,840'],
              ['beebeeb/desktop', 'Swift/C++/Rust', 'macOS, Windows, Linux', '18,406'],
              ['beebeeb/rclone-backend', 'Go', 'rclone plugin', '3,214'],
            ].map(([repo, lang, desc, stars], i, arr) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '1.2fr 0.6fr 1.4fr auto',
                alignItems: 'center', gap: 12,
                padding: '12px 18px',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              }}>
                <div className="t-mono" style={{ fontSize: 12.5, fontWeight: 600 }}>{repo}</div>
                <BBChip style={{ fontSize: 9.5, padding: '1px 7px' }}>{lang}</BBChip>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{desc}</div>
                <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>★ {stars}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </MarketingShell>
  );
}

// ─── LEGAL PAGE (warrant canary + transparency) ─────────────────────────────────────
function HiLegalPage() {
  return (
    <MarketingShell>
      <div style={{ padding: '64px 48px 40px', background: 'linear-gradient(180deg, var(--amber-bg), var(--paper) 85%)' }}>
        <div className="t-label" style={{ fontSize: 11, color: 'var(--amber-deep)' }}>Transparency · updated monthly</div>
        <div className="t-display" style={{ fontSize: 48, lineHeight: 1.05, marginTop: 12, maxWidth: 900 }}>
          We tell you what we can't tell you.
        </div>
        <div style={{ fontSize: 15, color: 'var(--ink-2)', marginTop: 18, maxWidth: 620, lineHeight: 1.65 }}>
          Warrant canary, transparency report, sub-processor list, and every government request we've received — to the extent the law allows us to disclose.
        </div>
      </div>

      {/* Canary */}
      <div style={{ padding: '32px 48px 48px', background: 'var(--paper)' }}>
        <div style={{
          background: 'var(--paper-2)', border: '1.5px solid var(--amber-deep)',
          borderRadius: 14, padding: 28,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
            <div style={{ width: 32, height: 32, borderRadius: 8, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Ico name="shield" size={15} color="var(--amber-deep)" />
            </div>
            <div>
              <div className="t-h2" style={{ fontSize: 20 }}>Warrant canary</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Signed by every director of Beebeeb GmbH. Updated the first Monday of each month.</div>
            </div>
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 6, alignItems: 'center' }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: 'oklch(0.65 0.14 155)' }} />
              <span className="t-mono" style={{ fontSize: 11, color: 'oklch(0.45 0.14 155)', fontWeight: 600 }}>CURRENT · last updated 2025-11-03</span>
            </div>
          </div>
          <div style={{ background: 'var(--paper)', borderRadius: 10, padding: 20, border: '1px solid var(--line)' }}>
            <div className="t-mono" style={{ fontSize: 12, lineHeight: 1.75, color: 'var(--ink-2)' }}>
              <p>As of <strong style={{ color: 'var(--ink)' }}>2025-11-03</strong>, Beebeeb GmbH has:</p>
              <ul style={{ marginTop: 10, marginLeft: 20, listStyle: 'disc' }}>
                <li>received <strong style={{ color: 'var(--ink)' }}>zero</strong> National Security Letters or equivalent;</li>
                <li>received <strong style={{ color: 'var(--ink)' }}>zero</strong> requests to insert a backdoor into any product;</li>
                <li>received <strong style={{ color: 'var(--ink)' }}>zero</strong> requests compelled by a gag order.</li>
              </ul>
              <p style={{ marginTop: 12 }}>Should any of the above cease to be true, this page will be removed and not replaced.</p>
              <div style={{ marginTop: 18, padding: 14, background: 'var(--paper-2)', borderRadius: 6, fontSize: 10.5, color: 'var(--ink-3)' }}>
                -----BEGIN PGP SIGNATURE-----<br/>
                iQIcBAEBCgAGBQJljW8mAAoJEHx9k... <span style={{ color: 'var(--ink-4)' }}>[8 lines truncated]</span><br/>
                =Qp1m<br/>
                -----END PGP SIGNATURE-----
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Transparency numbers */}
      <div style={{ padding: '32px 48px 56px', background: 'var(--paper)' }}>
        <div className="t-h2" style={{ fontSize: 24, marginBottom: 18 }}>Requests received — H1 2025</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 14 }}>
          {[
            ['Government requests', '4', 'All from EU member states, pursuant to MLAT.'],
            ['Data produced', 'Ciphertext only', 'Because that\'s all we have.'],
            ['Requests rejected', '1', 'Overbroad. Narrowed after negotiation.'],
            ['User notifications sent', '3', 'When not prohibited by law.'],
          ].map(([k, v, b]) => (
            <div key={k} style={{ padding: 20, background: 'var(--paper-2)', borderRadius: 12, border: '1px solid var(--line)' }}>
              <div className="t-label" style={{ fontSize: 10 }}>{k}</div>
              <div className="t-display" style={{ fontSize: 28, marginTop: 6 }}>{v}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 8, lineHeight: 1.5 }}>{b}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Sub-processors */}
      <div style={{ padding: '40px 48px 72px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
        <div className="t-h2" style={{ fontSize: 24, marginBottom: 8 }}>Sub-processors</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 20 }}>
          The full list of third parties that touch any of your data — which, for storage and compute, is ciphertext only.
        </div>
        <div style={{ background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line)', overflow: 'hidden' }}>
          <div style={{
            display: 'grid', gridTemplateColumns: '1.3fr 1fr 1.5fr 1fr',
            padding: '12px 22px', background: 'var(--paper-2)',
            borderBottom: '1px solid var(--line)',
            fontSize: 10.5, fontWeight: 600, color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.4,
          }}>
            <div>Vendor</div><div>Purpose</div><div>Data</div><div>Location</div>
          </div>
          {[
            ['Hetzner Online GmbH', 'Infrastructure', 'Ciphertext blobs', 'Germany'],
            ['Scaleway S.A.', 'Infrastructure (redundancy)', 'Ciphertext blobs', 'France'],
            ['LeaseWeb NL', 'Edge + CDN', 'Encrypted requests only', 'Netherlands'],
            ['Postmark by Wildbit', 'Transactional email', 'Email address', 'Germany (Frankfurt)'],
            ['Stripe Payments Europe', 'Billing', 'Billing info (never files)', 'Ireland'],
            ['Plausible Analytics', 'Marketing-site stats', 'Anonymous page-view', 'Germany'],
          ].map((row, i, arr) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '1.3fr 1fr 1.5fr 1fr',
              padding: '14px 22px', alignItems: 'center',
              borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              fontSize: 12.5,
            }}>
              <div style={{ fontWeight: 500 }}>{row[0]}</div>
              <div style={{ color: 'var(--ink-2)' }}>{row[1]}</div>
              <div className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>{row[2]}</div>
              <div style={{ color: 'var(--ink-2)' }}>{row[3]}</div>
            </div>
          ))}
        </div>
      </div>
    </MarketingShell>
  );
}

Object.assign(window, { HiSecurityPage, HiComparePage, HiAboutPage, HiDevelopersPage, HiLegalPage });
