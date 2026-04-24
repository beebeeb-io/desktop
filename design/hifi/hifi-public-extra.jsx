// hifi-public-extra.jsx — status page, docs landing, blog, changelog, careers, 404, 500, contact sales, bug bounty, audits, newsletter confirm

function PublicShell({ children, title }) {
  return (
    <div style={{ width: 1280, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-1)' }}>
      <div style={{ padding: '14px 32px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 18 }}>
        <BBLogo size={15} />
        <span className="t-micro" style={{ color: 'var(--ink-3)' }}>{title}</span>
        <span style={{ marginLeft: 'auto', display: 'flex', gap: 18, fontSize: 12.5 }}>
          {['Product', 'Security', 'Pricing', 'Docs', 'Status'].map(l => <a key={l} style={{ color: 'var(--ink-2)' }}>{l}</a>)}
          <BBBtn size="sm" variant="amber">Start free</BBBtn>
        </span>
      </div>
      {children}
    </div>
  );
}

function HiStatusPage() {
  const services = [
    { n: 'Web app', s: 'ok' },
    { n: 'API', s: 'ok' },
    { n: 'File storage · Amsterdam (primary)', s: 'ok' },
    { n: 'File storage · EU failover', s: 'ok' },
    { n: 'Desktop sync', s: 'ok' },
    { n: 'Mobile push', s: 'ok' },
    { n: 'Share links', s: 'ok' },
    { n: 'Billing · Stripe EU', s: 'ok' },
  ];
  return (
    <PublicShell title="status.beebeeb.io">
      <div style={{ padding: '40px 60px' }}>
        <div style={{ marginBottom: 14, padding: '8px 12px', background: 'var(--paper-2)', border: '1px dashed var(--line-2)', borderRadius: 8, fontSize: 11.5, color: 'var(--ink-3)', display: 'inline-flex', alignItems: 'center', gap: 8 }}>
          <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--amber-deep)' }} />
          Pre-launch preview · live metrics start at GA
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 6 }}>
          <span style={{ width: 12, height: 12, borderRadius: '50%', background: 'oklch(0.72 0.16 155)', boxShadow: '0 0 0 4px oklch(0.96 0.04 155)' }} />
          <div style={{ fontSize: 26, fontWeight: 600, letterSpacing: -0.3 }}>All systems operational</div>
        </div>
        <div className="t-body" style={{ fontSize: 13.5, color: 'var(--ink-2)', marginBottom: 28 }}>Staging environment · auto-refresh every 60s</div>
        <div style={{ border: '1px solid var(--line-2)', borderRadius: 10, overflow: 'hidden' }}>
          {services.map((s, i) => (
            <div key={s.n} style={{ padding: '14px 18px', borderBottom: i < services.length - 1 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 12 }}>
              <span style={{ width: 8, height: 8, borderRadius: '50%', background: s.s === 'ok' ? 'oklch(0.72 0.16 155)' : 'var(--amber-deep)' }} />
              <span style={{ fontSize: 13 }}>{s.n}</span>
              <span style={{ marginLeft: 'auto', display: 'flex', gap: 2 }}>
                {Array.from({length: 90}).map((_, j) => (
                  <span key={j} style={{ width: 3, height: 22, borderRadius: 1, background: (s.s === 'degraded' && j > 82) ? 'var(--amber-deep)' : 'oklch(0.72 0.16 155)' }} />
                ))}
              </span>
              <span className="t-micro" style={{ width: 52, textAlign: 'right', color: 'var(--ink-3)' }}>{s.s === 'ok' ? '100%' : '98.2%'}</span>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 30 }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Recent incidents</div>
          <div style={{ padding: '14px 16px', fontSize: 12.5, color: 'var(--ink-3)', background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10 }}>No incidents to report. Once we're live, every outage — including brief ones — will be posted here with a post-mortem.</div>
        </div>
        <div style={{ marginTop: 30, padding: 18, border: '1px solid var(--line-2)', borderRadius: 10, display: 'flex', alignItems: 'center', gap: 14 }}>
          <Ico name="shield" size={14} color="var(--ink-2)" />
          <div style={{ fontSize: 13 }}>
            Uptime target <strong>99.9%</strong> at GA · <a style={{ color: 'var(--amber-deep)' }}>subscribe to incidents via email or RSS</a>
          </div>
        </div>
      </div>
    </PublicShell>
  );
}

function HiDocsLanding() {
  return (
    <PublicShell title="docs.beebeeb.io">
      <div style={{ padding: '40px 60px' }}>
        <div style={{ fontSize: 32, fontWeight: 600, letterSpacing: -0.4 }}>Documentation</div>
        <div className="t-body" style={{ fontSize: 14, color: 'var(--ink-2)', marginTop: 6 }}>Everything about using Beebeeb — and how the encryption actually works.</div>
        <div style={{ marginTop: 24, padding: '12px 16px', border: '1px solid var(--line-2)', borderRadius: 10, background: 'var(--paper-2)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <Ico name="search" size={13} color="var(--ink-3)" />
          <span style={{ fontSize: 13.5, color: 'var(--ink-3)' }}>Search docs…</span>
          <span className="t-mono t-micro" style={{ marginLeft: 'auto', padding: '2px 6px', background: 'var(--paper)', border: '1px solid var(--line-2)', borderRadius: 4 }}>⌘ K</span>
        </div>
        <div style={{ marginTop: 32, display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 16 }}>
          {[
            { t: 'Getting started', d: 'First upload, recovery phrase, adding a device.', items: ['Quickstart', 'Installing the desktop app', 'Importing from Dropbox', 'Mobile apps'] },
            { t: 'Sharing & teams', d: 'Links, permissions, team keys.', items: ['Public links', 'Share with a person', 'Team workspaces', 'Rotating team keys'] },
            { t: 'Security & crypto', d: 'How zero-knowledge actually works.', items: ['Threat model', 'Key derivation (Argon2id)', 'File format', 'Audit reports'] },
            { t: 'Developers', d: 'API, CLI, SDK.', items: ['REST API', 'bb CLI', 'JS / Go / Python SDK', 'Webhooks'] },
            { t: 'Admin & compliance', d: 'SSO, audit, DPA.', items: ['SAML SSO setup', 'Audit log export', 'GDPR / DPA', 'Data export'] },
            { t: 'Billing', d: 'Plans, invoicing, VAT.', items: ['Upgrading & downgrading', 'Invoice delivery', 'VAT & reverse charge', 'Education discount'] },
          ].map((c, i) => (
            <div key={c.t} style={{ padding: 20, borderRadius: 10, border: '1px solid var(--line-2)', background: i === 2 ? 'var(--amber-bg)' : 'var(--paper)' }}>
              <div style={{ fontSize: 14, fontWeight: 600, display: 'flex', alignItems: 'center', gap: 8 }}>
                {i === 2 && <BBHex size={10} />}{c.t}
              </div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-2)', marginTop: 4, lineHeight: 1.5 }}>{c.d}</div>
              <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 5 }}>
                {c.items.map(it => (
                  <a key={it} style={{ fontSize: 12, color: 'var(--ink-2)', padding: '3px 0' }}>→ {it}</a>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </PublicShell>
  );
}

function HiChangelog() {
  const entries = [
    { d: 'Apr 22, 2026', tag: 'security', t: 'Post-quantum key exchange (X25519 + ML-KEM) in preview', b: 'New vaults can opt into hybrid key agreement. Existing vaults will be migrated automatically in Q3 — no action required. Details in docs.' },
    { d: 'Apr 15, 2026', tag: 'feature', t: 'File request links', b: 'You can now generate a public upload link that lets anyone send you files, encrypted to your key from the browser. They don\'t need a Beebeeb account.' },
    { d: 'Apr 8, 2026', tag: 'mobile', t: 'iOS 17.5 · share extension', b: 'Send any file to Beebeeb from Photos, Files, Mail, or Safari.' },
    { d: 'Mar 29, 2026', tag: 'fix', t: 'Large folder uploads > 100k files', b: 'Previously stalled around 65k. Now chunks in 2k batches and continues across sessions.' },
  ];
  const tagColor = { security: 'amber', feature: 'green', mobile: 'default', fix: 'default' };
  return (
    <PublicShell title="changelog.beebeeb.io">
      <div style={{ padding: '40px 60px', maxWidth: 860 }}>
        <div style={{ fontSize: 32, fontWeight: 600, letterSpacing: -0.4 }}>Changelog</div>
        <div className="t-body" style={{ fontSize: 14, color: 'var(--ink-2)', marginTop: 6 }}>What shipped, what changed, what broke. Subscribe via <a style={{ color: 'var(--amber-deep)' }}>RSS</a>.</div>
        <div style={{ marginTop: 32 }}>
          {entries.map((e, i) => (
            <div key={i} style={{ padding: '24px 0', borderBottom: i < entries.length - 1 ? '1px solid var(--line)' : 'none' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span className="t-mono t-micro" style={{ color: 'var(--ink-3)', width: 100 }}>{e.d}</span>
                <BBChip variant={tagColor[e.tag]}>{e.tag}</BBChip>
              </div>
              <div style={{ fontSize: 18, fontWeight: 600, marginTop: 10, letterSpacing: -0.2 }}>{e.t}</div>
              <div style={{ fontSize: 13.5, color: 'var(--ink-2)', marginTop: 6, lineHeight: 1.65 }}>{e.b}</div>
            </div>
          ))}
        </div>
      </div>
    </PublicShell>
  );
}

function HiCareers() {
  const roles = [
    { t: 'Founding backend engineer (Go)', loc: 'Remote EU · Wijchen 1d/wk', team: 'Platform', note: 'First hire' },
    { t: 'Founding client engineer (TS · Swift or Kotlin)', loc: 'Remote EU', team: 'Apps', note: 'First hire' },
  ];
  return (
    <PublicShell title="Careers">
      <div style={{ padding: '56px 60px' }}>
        <BBChip variant="amber">Hiring our first engineers · 2 roles</BBChip>
        <div style={{ fontSize: 40, fontWeight: 600, letterSpacing: -0.6, marginTop: 14, maxWidth: 720 }}>Join us at employee number one.</div>
        <div className="t-body" style={{ fontSize: 15, color: 'var(--ink-2)', marginTop: 10, maxWidth: 680, lineHeight: 1.6 }}>Beebeeb is two brothers in Wijchen — Bram (CEO) and Guus (CTO) Langelaar — building a zero-knowledge cloud for European SMBs. We're hiring the next two people. If you want to own whole systems, read every line of our code, and ship without process theatre, read on.</div>
        <div style={{ marginTop: 36, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12 }}>
          {[
            { l: '2 co-founders · hiring #1 and #2' },
            { l: 'Remote-first · EU-only by policy' },
            { l: 'Bootstrapped · no VC' },
            { l: 'Meaningful equity · honest cash' },
          ].map(s => (
            <div key={s.l} style={{ padding: 14, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10, fontSize: 12.5, color: 'var(--ink-2)', fontWeight: 500 }}>{s.l}</div>
          ))}
        </div>
        <div style={{ marginTop: 44 }}>
          <div className="t-label" style={{ marginBottom: 12 }}>Open roles</div>
          <div style={{ border: '1px solid var(--line-2)', borderRadius: 10, overflow: 'hidden' }}>
            {roles.map((r, i) => (
              <div key={r.t} style={{ padding: '18px 20px', borderBottom: i < roles.length - 1 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 14 }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 15, fontWeight: 600 }}>{r.t}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{r.team} · {r.note}</div>
                </div>
                <span style={{ fontSize: 12.5, color: 'var(--ink-2)' }}>{r.loc}</span>
                <BBBtn size="sm">View role →</BBBtn>
              </div>
            ))}
          </div>
        </div>
        <div style={{ marginTop: 40, padding: 22, border: '1px solid var(--line-2)', borderRadius: 12, background: 'var(--paper-2)' }}>
          <div className="t-label" style={{ marginBottom: 6 }}>Not a fit right now?</div>
          <div style={{ fontSize: 13.5 }}>Email <span className="t-mono" style={{ fontSize: 12 }}>contact@beebeeb.io</span> with what you'd want to build here. We read everything, and we remember.</div>
        </div>
      </div>
    </PublicShell>
  );
}

function HiError({ code }) {
  const data = code === 404 ? {
    t: "Couldn't find that page",
    d: "The file, folder, or URL might be private, moved, or never existed.",
    cta: 'Back to drive'
  } : {
    t: 'Something broke on our end',
    d: "Not your fault. Our team was notified automatically. Your files are safe — encryption means we couldn't touch them even during an outage.",
    cta: 'Try again'
  };
  return (
    <div style={{ width: 820, height: 520, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-1)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ textAlign: 'center', maxWidth: 440 }}>
        <div style={{ fontSize: 96, fontWeight: 700, letterSpacing: -2, color: 'var(--ink-4)', lineHeight: 1, fontFamily: 'var(--font-mono)' }}>{code}</div>
        <div style={{ fontSize: 22, fontWeight: 600, marginTop: 20, letterSpacing: -0.2 }}>{data.t}</div>
        <div className="t-body" style={{ fontSize: 13.5, color: 'var(--ink-2)', marginTop: 10, lineHeight: 1.6 }}>{data.d}</div>
        {code === 500 && (
          <div style={{ marginTop: 16, padding: 12, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 8, fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--ink-3)' }}>
            incident id: inc_7d9a42c3 · 23 Apr 2026 14:32 UTC
          </div>
        )}
        <div style={{ marginTop: 22, display: 'flex', gap: 10, justifyContent: 'center' }}>
          <BBBtn variant="amber">{data.cta}</BBBtn>
          <BBBtn variant="ghost">Status page →</BBBtn>
        </div>
      </div>
    </div>
  );
}

function HiContactSales() {
  return (
    <div style={{ width: 920, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)', display: 'grid', gridTemplateColumns: '1fr 1fr' }}>
      <div style={{ padding: 36, background: 'var(--ink)', color: 'var(--paper)' }}>
        <BBChip variant="amber">Business · 20+ seats</BBChip>
        <div style={{ fontSize: 24, fontWeight: 600, marginTop: 16, letterSpacing: -0.3, lineHeight: 1.2 }}>A sovereign cloud for your organisation.</div>
        <div style={{ fontSize: 13, color: 'oklch(0.78 0.01 70)', marginTop: 12, lineHeight: 1.6 }}>Talk to Bram (CEO) or Guus (CTO) about SSO, custom DPA, volume pricing, on-prem key escrow, and migration from Microsoft / Google / Dropbox.</div>
        <div style={{ marginTop: 26, display: 'flex', flexDirection: 'column', gap: 10, fontSize: 13, color: 'oklch(0.85 0.01 70)' }}>
          {['Custom DPA review (legal turnaround 48h)', 'Dedicated implementation engineer', 'SLA 99.95% · emergency response 15 min', 'Optional on-prem key storage'].map(l => (
            <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ width: 5, height: 5, borderRadius: '50%', background: 'var(--amber)' }} />{l}
            </div>
          ))}
        </div>
        <div style={{ marginTop: 28, paddingTop: 20, borderTop: '1px solid oklch(0.28 0.01 70)', fontSize: 12, color: 'oklch(0.75 0.01 70)', lineHeight: 1.6 }}>
          <div>Bram Langelaar · CEO</div>
          <div className="t-mono" style={{ marginTop: 4 }}>bram@beebeeb.io · contact@beebeeb.io</div>
        </div>
      </div>
      <div style={{ padding: 36 }}>
        <div style={{ fontSize: 18, fontWeight: 600 }}>Tell us a little</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4 }}>We reply within one business day · always from a human</div>
        <div style={{ marginTop: 20, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {[
            { l: 'Work email', v: 'dpo@example.com', type: 'email' },
            { l: 'Name', v: 'Sam van Dijk' },
            { l: 'Organization', v: 'Your company' },
            { l: 'Seats needed', v: '180' },
          ].map(f => (
            <div key={f.l}>
              <div className="t-label" style={{ marginBottom: 4 }}>{f.l}</div>
              <input defaultValue={f.v} style={{ width: '100%', padding: '9px 12px', border: '1px solid var(--line-2)', borderRadius: 7, fontSize: 13, fontFamily: f.type === 'email' ? 'var(--font-mono)' : 'inherit' }} />
            </div>
          ))}
          <div>
            <div className="t-label" style={{ marginBottom: 4 }}>What problem are you solving?</div>
            <textarea style={{ width: '100%', padding: '10px 12px', border: '1px solid var(--line-2)', borderRadius: 7, fontSize: 13, minHeight: 90, fontFamily: 'inherit', resize: 'vertical' }} defaultValue="Migrating off Microsoft SharePoint. Need NIS2-compliant E2EE for customer KYC documents. Currently ~2 TB." />
          </div>
          <BBBtn variant="amber" size="lg" style={{ justifyContent: 'center', marginTop: 6 }}>Request a call</BBBtn>
          <span className="t-micro" style={{ color: 'var(--ink-3)', textAlign: 'center', marginTop: -4 }}>No sales robots · no drip campaigns</span>
        </div>
      </div>
    </div>
  );
}

function HiBugBounty() {
  return (
    <PublicShell title="Bug bounty · responsible disclosure">
      <div style={{ padding: '44px 60px', maxWidth: 820 }}>
        <BBChip variant="green">Rewards up to €25,000</BBChip>
        <div style={{ fontSize: 32, fontWeight: 600, letterSpacing: -0.4, marginTop: 14 }}>Find a flaw. Get paid. Help us fix it.</div>
        <div className="t-body" style={{ fontSize: 14, color: 'var(--ink-2)', marginTop: 8, lineHeight: 1.65 }}>Security is the whole product. We run a continuous program with clear rules — no lawyers, no threats, just gratitude and money.</div>
        <div style={{ marginTop: 30, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
          {[
            { sev: 'Critical', amt: '€10,000 – €25,000', ex: 'Break zero-knowledge, key extraction' },
            { sev: 'High', amt: '€2,500 – €10,000', ex: 'Auth bypass, SSRF to keys' },
            { sev: 'Medium', amt: '€500 – €2,500', ex: 'IDOR, privilege escalation' },
            { sev: 'Low', amt: '€100 – €500', ex: 'Info disclosure, weak defaults' },
          ].map((t, i) => (
            <div key={t.sev} style={{ padding: 14, borderRadius: 10, background: i === 0 ? 'var(--amber-bg)' : 'var(--paper-2)', border: `1px solid ${i === 0 ? 'var(--amber-deep)' : 'var(--line)'}` }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>{t.sev}</div>
              <div style={{ fontSize: 14, fontWeight: 700, marginTop: 4, color: i === 0 ? 'var(--amber-deep)' : 'var(--ink)' }}>{t.amt}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 6, lineHeight: 1.5 }}>{t.ex}</div>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 30 }}>
          <div className="t-label" style={{ marginBottom: 10 }}>How to report</div>
          <div style={{ padding: 18, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10, fontFamily: 'var(--font-mono)', fontSize: 12.5, color: 'var(--ink-2)', lineHeight: 1.7 }}>
            <div>Email: security@beebeeb.io</div>
            <div>PGP:   4096R / 7D9A 42C3 FA1E 88B0 · <a style={{ color: 'var(--amber-deep)' }}>fingerprint</a></div>
            <div style={{ marginTop: 8, color: 'var(--ink-3)' }}># first reply within 24h · triage within 72h</div>
          </div>
        </div>
        <div style={{ marginTop: 24 }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Hall of fame</div>
          <div style={{ padding: 18, background: 'var(--paper-2)', border: '1px dashed var(--line-2)', borderRadius: 10, fontSize: 12.5, color: 'var(--ink-3)', lineHeight: 1.6 }}>
            Empty for now — the program opens at public launch. First researchers to submit valid findings are named here with their permission, and paid out in EUR via SEPA.
          </div>
        </div>
      </div>
    </PublicShell>
  );
}

function HiAuditsPage() {
  const audits = [
    { firm: 'Radically Open Security', date: 'Scheduled · Q3 2026', scope: 'Crypto core + web client', result: 'Pre-launch audit — report published verbatim', verified: false },
  ];
  return (
    <PublicShell title="Third-party security audits">
      <div style={{ padding: '44px 60px', maxWidth: 920 }}>
        <div style={{ fontSize: 32, fontWeight: 600, letterSpacing: -0.4 }}>Audits</div>
        <div className="t-body" style={{ fontSize: 14, color: 'var(--ink-2)', marginTop: 8, lineHeight: 1.65, maxWidth: 640 }}>Independent review isn't marketing. We've booked our first audit before public launch — full report goes up verbatim, including anything unflattering. After launch we'll commit to at least one per year.</div>
        <div style={{ marginTop: 28, border: '1px solid var(--line-2)', borderRadius: 10, overflow: 'hidden' }}>
          {audits.map((a, i) => (
            <div key={a.firm} style={{ padding: '18px 20px', borderBottom: i < audits.length - 1 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 16 }}>
              <div style={{ width: 40, height: 40, borderRadius: 8, background: 'var(--paper-2)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 14, fontWeight: 700, color: 'var(--ink-2)' }}>{a.firm[0]}</div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 14, fontWeight: 600 }}>{a.firm}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{a.scope} · {a.date}</div>
              </div>
              <span style={{ fontSize: 12.5, color: 'var(--ink-2)', width: 320 }}>{a.result}</span>
              <BBChip variant="amber">Scheduled</BBChip>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 28, padding: 18, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 10, fontSize: 13, color: 'var(--ink-2)', lineHeight: 1.6 }}>
          Roadmap transparency: once the first audit completes, every audit thereafter will be listed here with its full PDF report — including findings, CVSS scores, and our remediation notes. No cherry-picking.
        </div>
      </div>
    </PublicShell>
  );
}

function HiNewsletterConfirm() {
  return (
    <div style={{ width: 560, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)', padding: 40, textAlign: 'center' }}>
      <div style={{ width: 56, height: 56, margin: '0 auto', borderRadius: '50%', background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <Ico name="mail" size={22} color="var(--amber-deep)" />
      </div>
      <div style={{ fontSize: 22, fontWeight: 600, marginTop: 16, letterSpacing: -0.2 }}>Check your inbox</div>
      <div className="t-body" style={{ fontSize: 13.5, color: 'var(--ink-2)', marginTop: 8, lineHeight: 1.6 }}>
        We sent a confirmation to <span className="t-mono" style={{ fontSize: 12.5, padding: '1px 6px', background: 'var(--paper-2)', borderRadius: 4 }}>isa@example.eu</span><br />
        Click the link to subscribe to our monthly-ish changelog digest.
      </div>
      <div style={{ marginTop: 22, padding: 14, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10, fontSize: 12, color: 'var(--ink-3)', lineHeight: 1.55 }}>
        Didn't receive it? Check spam, or <a style={{ color: 'var(--amber-deep)' }}>resend</a>. This list is hosted in the EU and we'll never share your address.
      </div>
    </div>
  );
}

function HiCookieBanner() {
  return (
    <div style={{ width: 560, background: 'var(--ink)', color: 'var(--paper)', borderRadius: 12, padding: 18, boxShadow: 'var(--shadow-3)' }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>We use one cookie.</div>
          <div style={{ fontSize: 12.5, color: 'oklch(0.82 0.01 70)', marginTop: 4, lineHeight: 1.55 }}>A session cookie to keep you signed in. No trackers, no analytics, no ad-tech. That's it.</div>
        </div>
        <BBBtn size="sm" variant="amber">Understood</BBBtn>
      </div>
    </div>
  );
}

Object.assign(window, { HiStatusPage, HiDocsLanding, HiChangelog, HiCareers, HiError, HiContactSales, HiBugBounty, HiAuditsPage, HiNewsletterConfirm, HiCookieBanner });
