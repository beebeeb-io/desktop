// hifi-onboarding.jsx — Onboarding C hi-fi

const RECOVERY_WORDS_HI = [
  'hive', 'amber', 'pollen', 'clover',
  'meadow', 'linden', 'nectar', 'drift',
  'saffron', 'thistle', 'willow', 'bramble',
];

function HiOnboarding() {
  return (
    <div className="bb-card elevated" style={{ width: 820, fontFamily: 'var(--font-sans)', color: 'var(--ink)' }}>
      {/* header */}
      <div style={{ padding: '16px 24px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 16 }}>
        <BBLogo size={15} />
        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 6 }}>
          {[1, 2, 3, 4].map(i => (
            <div key={i} style={{
              width: 28, height: 3, borderRadius: 2,
              background: i <= 3 ? 'var(--ink)' : 'var(--paper-3)',
            }} />
          ))}
          <span className="t-label" style={{ marginLeft: 8 }}>3 / 4</span>
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1.2fr 1fr' }}>
        {/* LEFT — phrase */}
        <div style={{ padding: 32, borderRight: '1px solid var(--line)' }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Recovery phrase · 12 words</div>
          <div className="t-h1" style={{ marginBottom: 6 }}>Your master key, in words.</div>
          <div className="t-body" style={{ color: 'var(--ink-3)', marginBottom: 22 }}>
            These 12 words are the only way to recover your account. Write them down or save to a password manager.
          </div>

          <div style={{
            background: 'var(--paper-2)', border: '1px solid var(--line)',
            borderRadius: 'var(--r-3)', padding: 18, marginBottom: 16,
          }}>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '10px 28px' }}>
              {RECOVERY_WORDS_HI.map((w, i) => (
                <div key={i} style={{
                  display: 'flex', alignItems: 'baseline', gap: 10,
                  paddingBottom: 8, borderBottom: '1px dashed var(--line)',
                }}>
                  <span className="t-mono" style={{ color: 'var(--ink-4)', width: 18, fontSize: 11 }}>
                    {String(i + 1).padStart(2, '0')}
                  </span>
                  <span className="t-mono" style={{ fontSize: 14, fontWeight: 500 }}>{w}</span>
                </div>
              ))}
            </div>
          </div>

          <div style={{ display: 'flex', gap: 8 }}>
            <BBBtn size="sm" icon={<Ico name="copy" size={12} />}>Copy</BBBtn>
            <BBBtn size="sm" icon={<Ico name="download" size={12} />}>Download PDF</BBBtn>
            <BBBtn size="sm" variant="ghost">Print card</BBBtn>
          </div>
        </div>

        {/* RIGHT — why + CTA */}
        <div style={{ padding: 32, background: 'var(--paper-2)', display: 'flex', flexDirection: 'column' }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Why this matters</div>
          <div className="t-h3" style={{ marginBottom: 20, color: 'var(--ink)' }}>
            True zero-knowledge means we can't reach in — and neither can anyone else.
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginBottom: 24 }}>
            {[
              ['We can\'t see it', 'Encrypted on your device before upload.'],
              ['We can\'t reset it', 'Lost phrase = lost access. That\'s the deal.'],
              ['You\'re the only custodian', 'No backdoors, no master keys, no exceptions.'],
            ].map(([t, d], i) => (
              <div key={i} style={{ display: 'flex', gap: 11 }}>
                <div style={{
                  width: 22, height: 22, flexShrink: 0,
                  background: 'var(--amber-bg)', borderRadius: 999,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  color: 'var(--amber-deep)',
                }}>
                  <Ico name={i === 0 ? 'eye' : i === 1 ? 'key' : 'shield'} size={12} />
                </div>
                <div>
                  <div style={{ fontSize: 13, fontWeight: 600 }}>{t}</div>
                  <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginTop: 2 }}>{d}</div>
                </div>
              </div>
            ))}
          </div>

          <div style={{ marginTop: 'auto' }}>
            <label style={{ display: 'flex', gap: 10, fontSize: 13, color: 'var(--ink-2)', marginBottom: 16, cursor: 'pointer' }}>
              <BBCheck on />
              <span>I've saved my recovery phrase offline.</span>
            </label>
            <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }}>
              Continue <Ico name="chevRight" size={14} />
            </BBBtn>
            <div style={{ textAlign: 'center', marginTop: 10, fontSize: 11.5, color: 'var(--ink-4)' }}>
              <BBRegionBadge />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiOnboarding });
