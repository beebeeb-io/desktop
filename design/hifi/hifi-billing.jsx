// hifi-billing.jsx — Plans page, upgrade flow, billing panel

function PlanCard({ name, price, note, seat, storage, items, cta, ctaVariant, highlight, current }) {
  return (
    <div className="bb-card" style={{
      padding: 22, display: 'flex', flexDirection: 'column', gap: 14,
      position: 'relative', overflow: 'visible',
      borderColor: highlight ? 'oklch(0.78 0.14 80)' : 'var(--line-2)',
      background: highlight ? 'linear-gradient(180deg, var(--amber-bg), var(--paper))' : 'var(--paper)',
      boxShadow: highlight ? '0 12px 32px -12px oklch(0.78 0.17 80 / 0.35)' : 'var(--shadow-1)',
    }}>
      {highlight && (
        <div style={{
          position: 'absolute', top: -10, left: 22,
          padding: '3px 9px', borderRadius: 999,
          background: 'var(--ink)', color: 'var(--amber)',
          fontFamily: 'var(--font-mono)', fontSize: 10, letterSpacing: '0.08em',
          textTransform: 'uppercase', fontWeight: 600,
        }}>Most popular</div>
      )}
      {current && (
        <div style={{
          position: 'absolute', top: -10, right: 22,
          padding: '3px 9px', borderRadius: 999,
          background: 'oklch(0.94 0.06 155)', color: 'oklch(0.35 0.12 155)',
          border: '1px solid oklch(0.85 0.09 155)',
          fontFamily: 'var(--font-mono)', fontSize: 10, letterSpacing: '0.08em',
          textTransform: 'uppercase', fontWeight: 600,
        }}>Current</div>
      )}

      <div>
        <div className="t-label" style={{ marginBottom: 4 }}>{name}</div>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
          <div style={{ fontFamily: 'var(--font-sans)', fontSize: 30, fontWeight: 700, letterSpacing: '-0.025em' }}>{price}</div>
          {seat && <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{seat}</div>}
        </div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{note}</div>
      </div>

      <div style={{
        padding: '10px 12px', borderRadius: 'var(--r-2)',
        background: highlight ? 'var(--paper)' : 'var(--paper-2)',
        border: '1px solid var(--line)',
        display: 'flex', alignItems: 'center', gap: 8,
      }}>
        <Ico name="cloud" size={13} color="var(--amber-deep)" />
        <span className="t-mono" style={{ fontSize: 12, fontWeight: 500 }}>{storage}</span>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
        {items.map((it, i) => (
          <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'flex-start', fontSize: 12.5, color: 'var(--ink-2)' }}>
            <Ico name="check" size={11} color={it.strong ? 'var(--amber-deep)' : 'oklch(0.55 0.1 155)'} />
            <span style={{ lineHeight: 1.45, fontWeight: it.strong ? 500 : 400 }}>{it.label}</span>
          </div>
        ))}
      </div>

      <div style={{ marginTop: 'auto', paddingTop: 4 }}>
        <BBBtn variant={ctaVariant || 'default'} size="lg" style={{ width: '100%', justifyContent: 'center' }}>
          {cta}
        </BBBtn>
      </div>
    </div>
  );
}

function HiPricing() {
  const plans = [
    {
      name: 'Free',
      price: '€0',
      note: 'Forever. No card needed.',
      storage: '20 GB',
      cta: 'Create account',
      items: [
        { label: 'E2E encryption · zero-knowledge' },
        { label: 'Unlimited devices' },
        { label: 'Link sharing · passphrase · expiry' },
        { label: 'Photos & drive' },
      ],
    },
    {
      name: 'Personal',
      price: '€5',
      seat: '/ month',
      note: '2 TB. One flat price. No upsells.',
      storage: '2 TB',
      cta: 'Start 14-day trial',
      ctaVariant: 'default',
      items: [
        { label: 'Everything in Free' },
        { label: '2 TB encrypted storage', strong: true },
        { label: 'Photo library · EXIF stripped' },
        { label: 'Recovery via trusted contact' },
        { label: 'EU jurisdiction of choice' },
      ],
    },
    {
      name: 'Team',
      price: '€6',
      seat: '/ user · month',
      note: '3+ seats. 2 TB each, pooled.',
      storage: '2 TB / seat · pooled',
      cta: 'Try with your team',
      ctaVariant: 'amber',
      highlight: true,
      items: [
        { label: 'Everything in Personal' },
        { label: 'Shared vaults & team keys', strong: true },
        { label: 'Granular permissions + client portals', strong: true },
        { label: 'Audit log & activity export' },
        { label: 'Centralised billing, 1 invoice' },
      ],
    },
    {
      name: 'Business',
      price: '€12',
      seat: '/ user · month',
      note: 'Regulated industries & larger teams.',
      storage: '5 TB / seat · pooled',
      cta: 'Talk to sales',
      items: [
        { label: 'Everything in Team' },
        { label: 'SSO · SAML · SCIM', strong: true },
        { label: 'Compliance dashboard (NIS2, DORA)', strong: true },
        { label: 'Signed DPA + data residency choice' },
        { label: 'Priority support · 4h response' },
      ],
    },
  ];

  return (
    <div className="bb-card elevated" style={{ width: 1180, padding: 0, overflow: 'hidden' }}>
      {/* Header */}
      <div style={{ padding: '32px 36px 18px', textAlign: 'center', borderBottom: '1px solid var(--line)', background: 'var(--paper)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, justifyContent: 'center', marginBottom: 14 }}>
          <BBChip variant="amber"><Ico name="shield" size={10} /> Zero-knowledge</BBChip>
          <BBChip><span className="dot" style={{ background: 'oklch(0.72 0.16 155)' }} /> Priced in €, billed in €</BBChip>
          <BBRegionBadge region="EU-only" />
        </div>
        <div className="t-display" style={{ marginBottom: 8 }}>Honest storage, priced honestly.</div>
        <div style={{ fontSize: 14, color: 'var(--ink-3)', maxWidth: 640, margin: '0 auto' }}>
          No per-feature upsells, no data mined to subsidise the cheap tier, no EU-wash. Every plan is end-to-end encrypted and stored on EU soil.
        </div>

        {/* Toggle */}
        <div style={{
          display: 'inline-flex', marginTop: 18, padding: 3,
          background: 'var(--paper-2)', border: '1px solid var(--line)',
          borderRadius: 999,
        }}>
          <span style={{ padding: '6px 14px', borderRadius: 999, background: 'var(--paper)', boxShadow: 'var(--shadow-1)', fontSize: 12.5, fontWeight: 600 }}>Monthly</span>
          <span style={{ padding: '6px 14px', fontSize: 12.5, color: 'var(--ink-3)', display: 'flex', alignItems: 'center', gap: 6 }}>
            Yearly <BBChip variant="amber" style={{ fontSize: 9.5, padding: '1px 6px' }}>Save 20%</BBChip>
          </span>
        </div>
      </div>

      {/* Plans grid */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 14, padding: 28, background: 'var(--paper-2)' }}>
        {plans.map((p, i) => <PlanCard key={i} {...p} />)}
      </div>

      {/* Comparison strip */}
      <div style={{
        padding: '18px 36px', borderTop: '1px solid var(--line)',
        background: 'var(--paper)',
        display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 20,
      }}>
        {[
          ['shield', 'All plans E2E encrypted', 'AES-256-GCM · keys never leave your device'],
          ['cloud', 'EU-only infrastructure', 'Your choice: FRA · AMS · PAR'],
          ['users', '30-day refund', 'No questions, no retention calls'],
          ['key', 'Open-source client apps', 'Audit the code · GitHub'],
        ].map(([ico, t, s], i) => (
          <div key={i} style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}>
            <div style={{
              width: 24, height: 24, flexShrink: 0, borderRadius: 6,
              background: 'var(--amber-bg)', color: 'var(--amber-deep)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Ico name={ico} size={12} />
            </div>
            <div>
              <div style={{ fontSize: 12.5, fontWeight: 600 }}>{t}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{s}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Upgrade flow — in-app modal ────────────────────────────────
function HiUpgradeFlow() {
  return (
    <div className="bb-card elevated" style={{ width: 600, overflow: 'hidden' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div className="t-h3">Upgrade to Team</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>14-day free trial</BBChip>
      </div>

      <div style={{ padding: 22 }}>
        {/* Seats */}
        <div className="t-label" style={{ marginBottom: 8 }}>Seats</div>
        <div style={{
          display: 'flex', alignItems: 'center', gap: 12,
          padding: 14, background: 'var(--paper-2)',
          border: '1px solid var(--line)', borderRadius: 'var(--r-2)',
          marginBottom: 16,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <div style={{ width: 28, height: 28, borderRadius: 6, border: '1px solid var(--line-2)', background: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' }}>
              <Ico name="plus" size={11} color="var(--ink-3)" style={{ transform: 'rotate(45deg)' }} />
            </div>
            <div className="t-mono" style={{ fontSize: 18, fontWeight: 600, minWidth: 40, textAlign: 'center' }}>8</div>
            <div style={{ width: 28, height: 28, borderRadius: 6, border: '1px solid var(--line-2)', background: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' }}>
              <Ico name="plus" size={11} color="var(--ink-2)" />
            </div>
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 12.5, color: 'var(--ink-2)' }}>8 × €6 = <span className="t-mono" style={{ fontWeight: 600 }}>€48.00</span> / mo</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>16 TB shared pool · add or remove any time</div>
          </div>
        </div>

        {/* Cycle */}
        <div className="t-label" style={{ marginBottom: 8 }}>Billing cycle</div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 18 }}>
          <div style={{
            padding: 12, borderRadius: 'var(--r-2)',
            background: 'var(--paper)', border: '1px solid var(--line)',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
              <div style={{ width: 14, height: 14, borderRadius: 999, border: '1.5px solid var(--line-2)' }} />
              <span style={{ fontSize: 13, fontWeight: 500 }}>Monthly</span>
            </div>
            <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', paddingLeft: 22 }}>€48.00 / mo</div>
          </div>
          <div style={{
            padding: 12, borderRadius: 'var(--r-2)',
            background: 'var(--amber-bg)', border: '1.5px solid var(--amber-deep)',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
              <div style={{ width: 14, height: 14, borderRadius: 999, border: '1.5px solid var(--amber-deep)', background: 'var(--amber)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--ink)' }} />
              </div>
              <span style={{ fontSize: 13, fontWeight: 600 }}>Yearly</span>
              <BBChip variant="filled" style={{ marginLeft: 'auto', fontSize: 9.5 }}>Save €115.20</BBChip>
            </div>
            <div className="t-mono" style={{ fontSize: 11, color: 'oklch(0.35 0.1 72)', paddingLeft: 22 }}>€460.80 / yr · €38.40 / mo equiv.</div>
          </div>
        </div>

        {/* Data residency */}
        <div className="t-label" style={{ marginBottom: 8 }}>Data residency</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8, marginBottom: 18 }}>
          {[
            { city: 'Frankfurt', country: 'DE', op: 'Hetzner', sel: true },
            { city: 'Amsterdam', country: 'NL', op: 'Leaseweb', sel: false },
            { city: 'Paris', country: 'FR', op: 'Scaleway', sel: false },
          ].map((r, i) => (
            <div key={i} style={{
              padding: 10, borderRadius: 'var(--r-2)',
              background: r.sel ? 'var(--paper)' : 'var(--paper-2)',
              border: r.sel ? '1.5px solid var(--ink)' : '1px solid var(--line)',
            }}>
              <div style={{ fontSize: 12.5, fontWeight: 600 }}>{r.city} · {r.country}</div>
              <div className="t-mono t-micro" style={{ color: 'var(--ink-3)' }}>{r.op}</div>
            </div>
          ))}
        </div>

        {/* Summary */}
        <div style={{
          padding: 14, background: 'var(--ink)', color: 'var(--paper)',
          borderRadius: 'var(--r-2)', marginBottom: 12,
        }}>
          <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 2 }}>
            <span style={{ fontSize: 13, opacity: 0.7 }}>Today</span>
            <span className="t-mono" style={{ marginLeft: 'auto', fontSize: 16, fontWeight: 600, color: 'var(--amber)' }}>€0.00</span>
          </div>
          <div className="t-micro" style={{ opacity: 0.6 }}>14 days free · then €460.80 / year billed on 6 May 2026 · cancel anytime</div>
        </div>

        <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }}>
          Start free trial <Ico name="chevRight" size={13} />
        </BBBtn>
        <div style={{ fontSize: 11, color: 'var(--ink-4)', textAlign: 'center', marginTop: 8 }}>
          SEPA · card · invoice · PayPal · Bitcoin
        </div>
      </div>
    </div>
  );
}

// ── Billing management panel ──────────────────────────────────
function HiBilling() {
  const invoices = [
    { n: 'BB-2026-0042', date: 'Apr 6, 2026', amt: '€48.00', status: 'Paid' },
    { n: 'BB-2026-0031', date: 'Mar 6, 2026', amt: '€48.00', status: 'Paid' },
    { n: 'BB-2026-0019', date: 'Feb 6, 2026', amt: '€42.00', status: 'Paid' },
    { n: 'BB-2026-0008', date: 'Jan 6, 2026', amt: '€42.00', status: 'Paid' },
  ];
  return (
    <div className="bb-card elevated" style={{ width: 960, padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="key" size={14} />
        <div>
          <div className="t-h3">Billing</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Acme Studio · Team plan · 8 seats</div>
        </div>
        <BBBtn size="sm" style={{ marginLeft: 'auto' }} icon={<Ico name="settings" size={12} />}>Change plan</BBBtn>
      </div>

      <div style={{ padding: '22px 22px 8px', display: 'grid', gridTemplateColumns: '1.2fr 1fr', gap: 14 }}>
        {/* Plan summary */}
        <div className="bb-card" style={{ padding: 18, flexShrink: 0 }}>
          <div className="t-label" style={{ marginBottom: 6 }}>Current plan</div>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, marginBottom: 10 }}>
            <div className="t-h2" style={{ fontSize: 24 }}>Team</div>
            <BBChip variant="amber" style={{ fontSize: 9.5 }}>Yearly · -20%</BBChip>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <div>
              <div className="t-micro" style={{ color: 'var(--ink-4)' }}>Seats</div>
              <div className="t-mono" style={{ fontSize: 15, fontWeight: 600 }}>8 / 12</div>
              <div className="bb-progress" style={{ marginTop: 4 }}>
                <div style={{ width: '66%' }} />
              </div>
            </div>
            <div>
              <div className="t-micro" style={{ color: 'var(--ink-4)' }}>Storage</div>
              <div className="t-mono" style={{ fontSize: 15, fontWeight: 600 }}>6.2 / 16 TB</div>
              <div className="bb-progress" style={{ marginTop: 4 }}>
                <div style={{ width: '39%' }} />
              </div>
            </div>
          </div>
          <div style={{
            marginTop: 14, padding: '10px 12px',
            background: 'var(--paper-2)', border: '1px solid var(--line)',
            borderRadius: 'var(--r-2)', display: 'flex', alignItems: 'center', gap: 10, fontSize: 12,
          }}>
            <Ico name="clock" size={12} color="var(--ink-3)" />
            <span style={{ flex: 1 }}>Renews <strong>6 May 2027</strong> · €460.80 charged to SEPA</span>
          </div>
        </div>

        {/* Payment methods */}
        <div className="bb-card" style={{ padding: 18, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center' }}>
            <div className="t-label">Payment method</div>
            <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }} icon={<Ico name="plus" size={11} />}>Add</BBBtn>
          </div>
          <div style={{ marginTop: 10 }}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 12,
              padding: '10px 12px', borderRadius: 'var(--r-2)',
              border: '1px solid var(--line)', background: 'var(--paper-2)', marginBottom: 8,
            }}>
              <div style={{
                width: 34, height: 22, borderRadius: 4,
                background: 'linear-gradient(135deg, #0066ff, #00aaff)',
                color: 'white', fontSize: 9, fontWeight: 700, letterSpacing: '0.05em',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>SEPA</div>
              <div style={{ flex: 1 }}>
                <div className="t-mono" style={{ fontSize: 12, fontWeight: 500 }}>NL•• •••• •••• 4721</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Acme Studio B.V.</div>
              </div>
              <BBChip variant="green" style={{ fontSize: 9.5 }}>Default</BBChip>
            </div>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 12,
              padding: '10px 12px', borderRadius: 'var(--r-2)',
              border: '1px solid var(--line)',
            }}>
              <div style={{
                width: 34, height: 22, borderRadius: 4,
                background: 'var(--ink)', color: 'var(--amber)',
                fontSize: 9, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>VISA</div>
              <div style={{ flex: 1 }}>
                <div className="t-mono" style={{ fontSize: 12, fontWeight: 500 }}>•••• •••• •••• 8812</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Expires 07 / 28</div>
              </div>
              <BBBtn size="sm" variant="ghost" icon={<Ico name="more" size={13} />} />
            </div>
          </div>
        </div>
      </div>

      {/* Invoices */}
      <div style={{ padding: '14px 22px 4px', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div className="t-label">Invoices</div>
        <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }} icon={<Ico name="download" size={11} />}>Download all</BBBtn>
      </div>
      <div style={{ padding: '0 22px 22px' }}>
        <div style={{
          display: 'grid', gridTemplateColumns: '1.2fr 1fr 100px 100px 40px', gap: 14,
          padding: '8px 0', borderBottom: '1px solid var(--line)',
        }}>
          <span className="t-label">Number</span>
          <span className="t-label">Date</span>
          <span className="t-label">Amount</span>
          <span className="t-label">Status</span>
          <span />
        </div>
        {invoices.map((v, i) => (
          <div key={i} style={{
            display: 'grid', gridTemplateColumns: '1.2fr 1fr 100px 100px 40px', gap: 14,
            padding: '11px 0', borderBottom: '1px solid var(--line)', alignItems: 'center',
          }}>
            <span className="t-mono" style={{ fontSize: 12 }}>{v.n}</span>
            <span style={{ fontSize: 12.5, color: 'var(--ink-2)' }}>{v.date}</span>
            <span className="t-mono" style={{ fontSize: 12, fontWeight: 500 }}>{v.amt}</span>
            <span><BBChip variant="green" style={{ fontSize: 9.5 }}>{v.status}</BBChip></span>
            <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <BBBtn size="sm" variant="ghost" icon={<Ico name="download" size={12} />} />
            </div>
          </div>
        ))}
      </div>

      {/* Footer */}
      <div style={{
        padding: '12px 22px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)',
        display: 'flex', alignItems: 'center', gap: 14, fontSize: 11.5, color: 'var(--ink-3)',
      }}>
        <Ico name="shield" size={12} color="var(--amber-deep)" />
        <span>All invoices are VAT-compliant (EU · reverse charge). Signed DPA on file.</span>
        <span style={{ marginLeft: 'auto' }}>
          <BBBtn size="sm" variant="ghost">Download DPA</BBBtn>
        </span>
      </div>
    </div>
  );
}

Object.assign(window, { HiPricing, HiUpgradeFlow, HiBilling, HiInvoice });

// ── Invoice PDF example ───────────────────────────────────────
function HiInvoice() {
  const items = [
    { desc: 'Team plan · 8 seats · yearly', period: '6 May 2026 → 5 May 2027', qty: 8, unit: '€57.60', total: '€460.80' },
    { desc: 'Pro-rata credit · previous cycle', period: '', qty: 1, unit: '–€12.00', total: '–€12.00' },
  ];
  const subtotal = 448.80, vat = 0.00, total = 448.80; // EU reverse charge
  return (
    <div style={{
      width: 780, background: '#fff', color: '#111',
      padding: '40px 44px', borderRadius: 6,
      boxShadow: '0 24px 60px -20px rgba(0,0,0,0.22), 0 0 0 1px rgba(0,0,0,0.06)',
      fontFamily: 'var(--font-sans)', fontSize: 12.5, lineHeight: 1.5,
    }}>
      {/* header */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 24, marginBottom: 28 }}>
        <div style={{ flex: 1 }}>
          <BBLogo size={14} />
          <div style={{ marginTop: 16, fontFamily: 'var(--font-mono)', fontSize: 10.5, color: '#666', lineHeight: 1.55 }}>
            Beebeeb B.V.<br />
            Keizersgracht 391<br />
            1016 EJ Amsterdam · Netherlands<br />
            KvK 98274610 · VAT NL 8.... B01
          </div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <div style={{ fontSize: 26, fontWeight: 700, letterSpacing: '-0.02em', marginBottom: 4 }}>Invoice</div>
          <div style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: '#666' }}>
            № BB-2026-0042<br />
            Issued 6 Apr 2026 · Due on receipt<br />
            Period 6 May 2026 → 5 May 2027
          </div>
        </div>
      </div>

      {/* billed to */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 24, padding: '14px 16px', background: '#faf8f4', borderRadius: 6, border: '1px solid #eee6d3', marginBottom: 24 }}>
        <div>
          <div style={{ fontSize: 9.5, letterSpacing: '0.12em', textTransform: 'uppercase', color: '#888', marginBottom: 4, fontWeight: 600 }}>Billed to</div>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Acme Studio B.V.</div>
          <div style={{ fontSize: 11.5, color: '#555' }}>
            Prinsengracht 42, 1015 DX Amsterdam · NL<br />
            VAT NL 859473281 B01 <span style={{ color: 'var(--amber-deep)' }}>· reverse charge</span><br />
            billing@brandstudio.nl
          </div>
        </div>
        <div>
          <div style={{ fontSize: 9.5, letterSpacing: '0.12em', textTransform: 'uppercase', color: '#888', marginBottom: 4, fontWeight: 600 }}>Payment</div>
          <div style={{ fontSize: 11.5, color: '#555', fontFamily: 'var(--font-mono)' }}>
            SEPA Direct Debit<br />
            Mandate BB-2025-0042-NL4721<br />
            Executed 6 Apr 2026 · 09:14 UTC
          </div>
        </div>
      </div>

      {/* line items */}
      <div style={{ marginBottom: 20 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '2.6fr 90px 90px 100px', fontSize: 10, letterSpacing: '0.1em', textTransform: 'uppercase', color: '#999', fontWeight: 600, padding: '8px 0', borderBottom: '2px solid #111' }}>
          <span>Description</span>
          <span style={{ textAlign: 'right' }}>Qty</span>
          <span style={{ textAlign: 'right' }}>Unit</span>
          <span style={{ textAlign: 'right' }}>Total</span>
        </div>
        {items.map((it, i) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '2.6fr 90px 90px 100px', padding: '14px 0', borderBottom: '1px solid #eee', alignItems: 'flex-start' }}>
            <div>
              <div style={{ fontSize: 13, fontWeight: 500 }}>{it.desc}</div>
              {it.period && <div style={{ fontSize: 11, color: '#888', fontFamily: 'var(--font-mono)', marginTop: 2 }}>{it.period}</div>}
            </div>
            <span style={{ textAlign: 'right', fontFamily: 'var(--font-mono)' }}>{it.qty}</span>
            <span style={{ textAlign: 'right', fontFamily: 'var(--font-mono)' }}>{it.unit}</span>
            <span style={{ textAlign: 'right', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{it.total}</span>
          </div>
        ))}
      </div>

      {/* totals */}
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 24 }}>
        <div style={{ width: 280 }}>
          <TotalRow label="Subtotal" value={`€${subtotal.toFixed(2)}`} />
          <TotalRow label="VAT (0% · NL reverse charge)" value={`€${vat.toFixed(2)}`} />
          <div style={{ height: 1, background: '#111', margin: '8px 0' }} />
          <TotalRow label="Total due" value={`€${total.toFixed(2)}`} bold large />
          <TotalRow label="Paid on 6 Apr 2026" value="€0.00" muted />
          <div style={{ marginTop: 8, padding: '8px 10px', background: '#f0f9ed', borderRadius: 4, fontSize: 11.5, color: '#2d6b1f', fontWeight: 500, textAlign: 'center' }}>
            ✓ Settled in full
          </div>
        </div>
      </div>

      {/* cryptographic receipt */}
      <div style={{ padding: 14, border: '1px dashed #ccc', borderRadius: 6, marginBottom: 20 }}>
        <div style={{ fontSize: 9.5, letterSpacing: '0.12em', textTransform: 'uppercase', color: '#888', marginBottom: 6, fontWeight: 600 }}>Tamper-evident receipt</div>
        <div style={{ fontFamily: 'var(--font-mono)', fontSize: 10, color: '#666', lineHeight: 1.6 }}>
          sha256 &nbsp;8b4e2f1a 9c3d5e7f b0a6d421 3f8c1e94 7a2b5d68 c1f9e03a 4d7b2c5e 9f1a8b26<br />
          signed &nbsp;2026-04-06T09:14:22Z &nbsp;· &nbsp;Beebeeb-Billing-2026-04-K1 (Ed25519)<br />
          verify &nbsp;<span style={{ color: 'var(--amber-deep)' }}>bb invoice verify BB-2026-0042</span>
        </div>
      </div>

      {/* footer */}
      <div style={{ display: 'flex', justifyContent: 'space-between', paddingTop: 16, borderTop: '1px solid #eee', fontSize: 10.5, color: '#888' }}>
        <span>Questions? billing@beebeeb.io · beebeeb.io/invoices</span>
        <span>Page 1 of 1</span>
      </div>
    </div>
  );
}

function TotalRow({ label, value, bold, large, muted }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 0', fontSize: large ? 14 : 12, fontWeight: bold ? 700 : 400, color: muted ? '#888' : '#111' }}>
      <span>{label}</span>
      <span style={{ fontFamily: 'var(--font-mono)' }}>{value}</span>
    </div>
  );
}
