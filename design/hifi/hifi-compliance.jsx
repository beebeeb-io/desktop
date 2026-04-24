// hifi-compliance.jsx — Compliance dashboard (Business tier)

function ComplianceTile({ framework, score, status, controls, icon, highlight }) {
  const tone = status === 'ok' ? 'oklch(0.45 0.12 155)'
    : status === 'warn' ? 'var(--amber-deep)'
    : 'var(--red)';
  const toneBg = status === 'ok' ? 'oklch(0.94 0.06 155)'
    : status === 'warn' ? 'var(--amber-bg)'
    : 'oklch(0.97 0.03 25)';
  return (
    <div className="bb-card" style={{ padding: 16, flexShrink: 0, borderColor: highlight ? 'var(--line-2)' : 'var(--line)' }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 12 }}>
        <div style={{
          width: 30, height: 30, borderRadius: 8, flexShrink: 0,
          background: toneBg, color: tone,
          border: '1px solid', borderColor: tone,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          opacity: 0.9,
        }}>
          <Ico name={icon} size={14} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>{framework}</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{controls} controls mapped</div>
        </div>
        <div className="t-mono" style={{ fontSize: 16, fontWeight: 700, color: tone }}>{score}%</div>
      </div>
      <div className="bb-progress" style={{ marginBottom: 8 }}>
        <div style={{ width: `${score}%`, background: tone }} />
      </div>
      <div style={{ fontSize: 11, color: 'var(--ink-3)', display: 'flex', alignItems: 'center', gap: 4 }}>
        <Ico name="check" size={10} color={tone} /> Last audit 3 Apr · next 6 Jul
      </div>
    </div>
  );
}

function HiCompliance() {
  const events = [
    { actor: 'anna@example.eu', action: 'Exported audit log', target: 'Apr 2026 · signed PDF', when: '4m ago', sev: 'info' },
    { actor: 'system', action: 'Sub-processor review completed', target: 'Hetzner Online GmbH', when: '2h ago', sev: 'ok' },
    { actor: 'marc@example.eu', action: 'Invited external viewer', target: 'Q2-financials · 24h expiry', when: '5h ago', sev: 'info' },
    { actor: 'system', action: 'Key rotation completed', target: 'Acme Studio vault · 128 files re-encrypted', when: 'yesterday', sev: 'ok' },
    { actor: 'anna@example.eu', action: 'Access request denied', target: 'jordan@example.eu · not on DPA', when: '2d ago', sev: 'warn' },
    { actor: 'system', action: 'Warrant canary updated', target: '22 Apr 2026 statement published', when: '3d ago', sev: 'info' },
  ];

  return (
    <div className="bb-card elevated" style={{ width: 1180, display: 'flex', overflow: 'hidden' }}>
      {/* Sidebar */}
      <div style={{ width: 220, borderRight: '1px solid var(--line)', background: 'var(--paper-2)' }}>
        <div style={{ padding: '16px 16px 10px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Ico name="shield" size={13} color="var(--ink-3)" />
          <div className="t-h3" style={{ fontSize: 14 }}>Compliance</div>
          <BBChip variant="amber" style={{ marginLeft: 'auto', fontSize: 9 }}>Business</BBChip>
        </div>
        <div style={{ padding: '6px 12px' }}>
          {[
            ['star', 'Overview', true],
            ['shield', 'Frameworks', false],
            ['clock', 'Audit log', false],
            ['users', 'Access requests', false, '3'],
            ['cloud', 'Sub-processors', false],
            ['key', 'Encryption keys', false],
            ['file', 'Documents & DPA', false],
            ['settings', 'Policies', false],
          ].map(([ico, l, act, b], i) => (
            <div key={i} className={'bb-side-item' + (act ? ' active' : '')}>
              <span className="bb-side-icon"><Ico name={ico} size={12} /></span>
              <span style={{ flex: 1 }}>{l}</span>
              {b && <span className="t-mono" style={{ fontSize: 10, color: 'var(--amber-deep)' }}>{b}</span>}
            </div>
          ))}
        </div>
        <div className="bb-divider" style={{ margin: '10px 16px' }} />
        <div style={{ padding: '0 16px 16px' }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Transparency</div>
          <div style={{
            padding: 10, background: 'var(--paper)', border: '1px solid var(--line)',
            borderRadius: 'var(--r-2)', fontSize: 11,
          }}>
            <div style={{ color: 'var(--ink-3)', marginBottom: 2 }}>Gov. requests received</div>
            <div className="t-mono" style={{ fontSize: 20, fontWeight: 700, color: 'var(--ink)' }}>0</div>
            <div className="t-micro" style={{ color: 'var(--ink-4)' }}>since Jan 2024</div>
          </div>
        </div>
      </div>

      {/* Main */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
        <div style={{ padding: '14px 24px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div className="t-h2">Compliance overview</div>
              <BBChip variant="green"><span className="dot" style={{ background: 'oklch(0.72 0.16 155)' }} /> Posture strong</BBChip>
            </div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>
              Acme Studio B.V. · 8 seats · live status across 4 frameworks
            </div>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <BBBtn size="sm" icon={<Ico name="download" size={12} />}>Evidence bundle</BBBtn>
            <BBBtn size="sm" variant="amber" icon={<Ico name="file" size={12} />}>Export compliance PDF</BBBtn>
          </div>
        </div>

        <div className="bb-scroll" style={{ padding: '20px 24px 28px', display: 'flex', flexDirection: 'column', gap: 18 }}>
          {/* Framework tiles */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12 }}>
            <ComplianceTile framework="GDPR" icon="shield" score={100} status="ok" controls={47} highlight />
            <ComplianceTile framework="NIS2" icon="lock" score={92} status="ok" controls={32} />
            <ComplianceTile framework="DORA" icon="cloud" score={88} status="warn" controls={24} />
            <ComplianceTile framework="ISO 27001" icon="key" score={95} status="ok" controls={114} />
          </div>

          {/* Two-up: data residency + access requests */}
          <div style={{ display: 'grid', gridTemplateColumns: '1.3fr 1fr', gap: 14 }}>
            {/* Data residency map */}
            <div className="bb-card" style={{ padding: 0, flexShrink: 0 }}>
              <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
                <Ico name="cloud" size={13} />
                <div className="t-h3" style={{ fontSize: 13 }}>Where your data lives — right now</div>
                <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }}>Change residency</BBBtn>
              </div>
              {/* Abstract EU map */}
              <div style={{ position: 'relative', height: 220, background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', overflow: 'hidden' }}>
                <div className="bb-honeycomb" style={{ position: 'absolute', inset: 0, opacity: 0.4 }} />
                <svg viewBox="0 0 400 220" style={{ position: 'absolute', inset: 0, width: '100%', height: '100%' }}>
                  {/* Minimal EU outline — abstract */}
                  <path d="M60 80 Q 80 40 130 50 Q 180 30 230 60 Q 290 40 330 80 Q 350 120 320 160 Q 280 190 220 180 Q 160 200 110 180 Q 70 160 60 120 Z"
                    fill="oklch(0.96 0.012 82)" stroke="var(--line-2)" strokeWidth="1" />
                  {/* Nodes */}
                  {[
                    { x: 190, y: 95, city: 'Frankfurt', pct: 72, active: true },
                    { x: 160, y: 80, city: 'Amsterdam', pct: 22, active: true },
                    { x: 140, y: 115, city: 'Paris', pct: 6, active: true },
                  ].map((n, i) => (
                    <g key={i}>
                      <circle cx={n.x} cy={n.y} r="16" fill="var(--amber-bg)" opacity="0.6" />
                      <circle cx={n.x} cy={n.y} r="8" fill="var(--amber)" stroke="var(--ink)" strokeWidth="1.3" />
                      <circle cx={n.x} cy={n.y} r="3" fill="var(--ink)" />
                      <text x={n.x} y={n.y - 22} textAnchor="middle" fontFamily="ui-monospace, monospace" fontSize="9" fill="var(--ink-2)" fontWeight="600">{n.city}</text>
                      <text x={n.x} y={n.y + 28} textAnchor="middle" fontFamily="ui-monospace, monospace" fontSize="10" fill="var(--ink)" fontWeight="700">{n.pct}%</text>
                    </g>
                  ))}
                </svg>
              </div>
              <div style={{ padding: '10px 16px', display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12 }}>
                {[
                  ['Frankfurt · DE', '4.48 TB', 'Hetzner'],
                  ['Amsterdam · NL', '1.37 TB', 'Leaseweb'],
                  ['Paris · FR', '0.37 TB', 'Scaleway'],
                ].map(([c, s, op], i) => (
                  <div key={i}>
                    <div style={{ fontSize: 12, fontWeight: 600 }}>{c}</div>
                    <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-2)' }}>{s}</div>
                    <div className="t-micro" style={{ color: 'var(--ink-4)' }}>{op}</div>
                  </div>
                ))}
              </div>
            </div>

            {/* Access requests */}
            <div className="bb-card" style={{ padding: 0, flexShrink: 0 }}>
              <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
                <Ico name="users" size={13} />
                <div className="t-h3" style={{ fontSize: 13 }}>GDPR access requests</div>
                <BBChip style={{ marginLeft: 'auto', fontSize: 9.5, color: 'var(--amber-deep)', borderColor: 'oklch(0.86 0.07 90)' }}>3 open</BBChip>
              </div>
              {[
                { who: 'data-subject@example.eu', type: 'Art. 15 · access', due: '4 days', p: 'warn' },
                { who: 'former-client@law.eu', type: 'Art. 17 · erasure', due: '11 days', p: 'info' },
                { who: 'contractor@design.nl', type: 'Art. 20 · portability', due: '18 days', p: 'info' },
              ].map((r, i, arr) => (
                <div key={i} style={{
                  padding: '11px 16px',
                  borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                  display: 'flex', alignItems: 'center', gap: 10,
                }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div className="t-mono" style={{ fontSize: 11.5, fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{r.who}</div>
                    <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{r.type}</div>
                  </div>
                  <span className="t-mono" style={{
                    fontSize: 10.5, padding: '2px 7px', borderRadius: 4,
                    background: r.p === 'warn' ? 'var(--amber-bg)' : 'var(--paper-2)',
                    color: r.p === 'warn' ? 'var(--amber-deep)' : 'var(--ink-3)',
                    border: '1px solid', borderColor: r.p === 'warn' ? 'oklch(0.86 0.07 90)' : 'var(--line)',
                    fontWeight: 600,
                  }}>{r.due}</span>
                </div>
              ))}
              <div style={{ padding: '8px 16px', textAlign: 'center', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', fontSize: 11.5 }}>
                <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>Handle requests →</span>
              </div>
            </div>
          </div>

          {/* Sub-processors */}
          <div className="bb-card" style={{ flexShrink: 0 }}>
            <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Ico name="link" size={13} />
              <div className="t-h3" style={{ fontSize: 13 }}>Sub-processors</div>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-4)' }}>4 active · all EU</span>
              <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }} icon={<Ico name="download" size={11} />}>Download list</BBBtn>
            </div>
            <div style={{
              display: 'grid', gridTemplateColumns: '1.4fr 1fr 1fr 1fr 90px', gap: 14,
              padding: '10px 16px', borderBottom: '1px solid var(--line)', background: 'var(--paper-2)',
            }}>
              {['Vendor', 'Purpose', 'Jurisdiction', 'Last review', ''].map(h => <span key={h} className="t-label">{h}</span>)}
            </div>
            {[
              ['Hetzner Online GmbH', 'Object storage', 'DE', '2h ago', 'ok'],
              ['Leaseweb B.V.', 'Object storage', 'NL', '6d ago', 'ok'],
              ['Scaleway SAS', 'Object storage', 'FR', '6d ago', 'ok'],
              ['Stripe Payments Europe', 'Billing', 'IE', '11d ago', 'ok'],
            ].map((r, i, arr) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '1.4fr 1fr 1fr 1fr 90px', gap: 14,
                padding: '11px 16px', alignItems: 'center',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              }}>
                <span style={{ fontSize: 12.5, fontWeight: 500 }}>{r[0]}</span>
                <span className="t-micro" style={{ color: 'var(--ink-2)' }}>{r[1]}</span>
                <span className="t-mono" style={{ fontSize: 11 }}>{r[2]}</span>
                <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r[3]}</span>
                <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
                  <BBChip variant="green" style={{ fontSize: 9.5 }}>Signed DPA</BBChip>
                </div>
              </div>
            ))}
          </div>

          {/* Audit log + Warrant canary */}
          <div style={{ display: 'grid', gridTemplateColumns: '1.5fr 1fr', gap: 14 }}>
            <div className="bb-card" style={{ flexShrink: 0 }}>
              <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
                <Ico name="clock" size={13} />
                <div className="t-h3" style={{ fontSize: 13 }}>Audit log · live</div>
                <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }} icon={<Ico name="download" size={11} />}>Signed export</BBBtn>
              </div>
              {events.map((e, i, arr) => {
                const sevC = e.sev === 'ok' ? 'oklch(0.55 0.12 155)'
                  : e.sev === 'warn' ? 'var(--amber-deep)'
                  : 'var(--ink-3)';
                return (
                  <div key={i} style={{
                    padding: '10px 16px',
                    borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                    display: 'flex', alignItems: 'center', gap: 10,
                  }}>
                    <span style={{ width: 6, height: 6, borderRadius: 999, background: sevC, flexShrink: 0 }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 12.5 }}>
                        <span className="t-mono" style={{ color: 'var(--ink-3)', fontSize: 11 }}>{e.actor}</span>
                        {' '}<span style={{ fontWeight: 500 }}>{e.action}</span>
                      </div>
                      <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{e.target}</div>
                    </div>
                    <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)' }}>{e.when}</span>
                  </div>
                );
              })}
            </div>

            <div className="bb-card" style={{ padding: 16, flexShrink: 0, background: 'var(--ink)', color: 'var(--paper)', borderColor: 'var(--ink)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
                <Ico name="shield" size={13} color="var(--amber)" />
                <div className="t-h3" style={{ fontSize: 13, color: 'var(--paper)' }}>Warrant canary</div>
                <BBChip style={{ marginLeft: 'auto', fontSize: 9.5, background: 'oklch(0.22 0.01 80)', color: 'var(--amber)', borderColor: 'oklch(0.28 0.01 80)' }}>Alive</BBChip>
              </div>
              <div style={{ fontSize: 12, opacity: 0.78, lineHeight: 1.55, marginBottom: 12 }}>
                As of 22 April 2026, Beebeeb (operated by Initlabs B.V., KvK 95157565) has received <strong style={{ color: 'var(--amber)' }}>zero</strong> gag orders, national security letters, or secret production requests.
              </div>
              <div style={{
                padding: 10, borderRadius: 'var(--r-2)',
                background: 'oklch(0.22 0.01 80)', border: '1px solid oklch(0.28 0.01 80)',
                fontFamily: 'var(--font-mono)', fontSize: 10, color: 'oklch(0.8 0.02 82)',
                wordBreak: 'break-all', lineHeight: 1.6,
              }}>
                sha256 · 4f9e…a20b<br />
                signed · 22 Apr 2026 09:00 UTC<br />
                notaries · 3 / 3
              </div>
              <BBBtn size="sm" variant="ghost" style={{ marginTop: 10, color: 'var(--amber)', borderColor: 'oklch(0.3 0.01 80)' }}>
                View signed statement →
              </BBBtn>
            </div>
          </div>

          {/* DPA + documents */}
          <div className="bb-card" style={{ padding: '14px 16px', display: 'flex', alignItems: 'center', gap: 14, flexShrink: 0 }}>
            <Ico name="file" size={14} color="var(--amber-deep)" />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>DPA signed · v2.3</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Executed 6 May 2025 · Anna Kovač (Acme Studio) · Bram Langelaar, CEO (Beebeeb)</div>
            </div>
            <BBBtn size="sm" variant="ghost" icon={<Ico name="download" size={11} />}>Download</BBBtn>
            <BBBtn size="sm" variant="ghost">View history</BBBtn>
          </div>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiCompliance });
