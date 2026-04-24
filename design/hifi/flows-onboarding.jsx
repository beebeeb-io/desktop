// onboarding.jsx — Signup + E2EE recovery phrase flow, 3 variants

// Recovery words for the mockups (clearly placeholder — not a real phrase)
const RECOVERY_WORDS = [
  'hive', 'amber', 'pollen', 'clover',
  'meadow', 'linden', 'nectar', 'drift',
  'saffron', 'thistle', 'willow', 'bramble',
];

// ─── Variant A — Single column, progressive disclosure ─────────────────
function OnboardingA() {
  return (
    <div className="wf-card" style={{ width: 520, padding: 28, fontFamily: 'var(--sans)' }}>
      <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 20 }}>
        <Logo size={16} />
        <span className="wf-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>Step 2 of 3</span>
      </div>

      <div style={{ height: 4, background: 'var(--paper-2)', border: '1px solid var(--ink)', marginBottom: 24 }}>
        <div style={{ width: '66%', height: '100%', background: 'var(--amber)' }} />
      </div>

      <h2 style={{ fontSize: 22, margin: '0 0 8px', fontWeight: 600, letterSpacing: '-0.01em' }}>
        Your recovery phrase
      </h2>
      <p style={{ fontSize: 13, color: 'var(--ink-2)', margin: '0 0 18px', lineHeight: 1.5 }}>
        These 12 words are the only way to recover your account. We can't see them
        and we can't reset them. Write them down, store them offline.
      </p>

      <div style={{
        display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8,
        padding: 16, border: '1.2px solid var(--ink)', background: 'var(--paper-2)',
        marginBottom: 16,
      }}>
        {RECOVERY_WORDS.map((w, i) => (
          <div key={i} style={{
            display: 'flex', alignItems: 'baseline', gap: 6,
            fontFamily: 'var(--mono)', fontSize: 12,
          }}>
            <span style={{ color: 'var(--ink-4)', fontSize: 10, width: 14 }}>{i + 1}</span>
            <span style={{ fontWeight: 500 }}>{w}</span>
          </div>
        ))}
      </div>

      <div className="wf-row" style={{ gap: 8, marginBottom: 18 }}>
        <Btn alt small>⧉ Copy</Btn>
        <Btn alt small>↓ Download PDF</Btn>
        <Btn alt small>◇ Print</Btn>
      </div>

      <label style={{ display: 'flex', gap: 8, fontSize: 12, color: 'var(--ink-2)', marginBottom: 18 }}>
        <span style={{ width: 14, height: 14, border: '1.2px solid var(--ink)', display: 'inline-block', flexShrink: 0 }} />
        I've stored my recovery phrase somewhere safe.
      </label>

      <Btn amber style={{ width: '100%', justifyContent: 'center', padding: '10px 12px' }}>
        Continue →
      </Btn>
    </div>
  );
}

// ─── Variant B — Honeycomb grid of words ───────────────────────────────
function OnboardingB() {
  return (
    <div className="wf-card" style={{ width: 520, padding: 0, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-hex" style={{ padding: '20px 24px', borderBottom: '1.2px solid var(--ink)' }}>
        <Logo size={16} />
        <div style={{ fontFamily: 'var(--hand)', fontSize: 22, marginTop: 14, color: 'var(--ink)', lineHeight: 1.1 }}>
          Welcome to the hive.
        </div>
        <div style={{ fontSize: 12, color: 'var(--ink-2)', marginTop: 4 }}>
          Meet your recovery key — the one thing we can't replace.
        </div>
      </div>

      <div style={{ padding: 22 }}>
        {/* Honeycomb layout — 4 per row, offset alternate rows */}
        <div style={{ marginBottom: 16 }}>
          {[0, 1, 2].map((row) => (
            <div key={row} style={{
              display: 'flex', gap: 6, justifyContent: 'center',
              marginLeft: row % 2 ? 26 : 0,
              marginTop: row === 0 ? 0 : -6,
            }}>
              {RECOVERY_WORDS.slice(row * 4, row * 4 + 4).map((w, i) => (
                <div key={i} style={{
                  width: 92, height: 52, background: 'var(--paper-2)',
                  clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)',
                  display: 'flex', flexDirection: 'column', alignItems: 'center',
                  justifyContent: 'center', fontFamily: 'var(--mono)', fontSize: 11,
                  border: '1px solid var(--ink)',
                }}>
                  <span style={{ fontSize: 9, color: 'var(--ink-3)' }}>{row * 4 + i + 1}</span>
                  <span style={{ fontWeight: 600 }}>{w}</span>
                </div>
              ))}
            </div>
          ))}
        </div>

        <div className="wf-row" style={{ justifyContent: 'center', gap: 8, marginBottom: 16 }}>
          <Btn alt small>⧉ Copy</Btn>
          <Btn alt small>↓ Save .txt</Btn>
          <Btn alt small>◇ Print card</Btn>
        </div>

        <div className="wf-annot" style={{ textAlign: 'center', marginBottom: 16, fontSize: 13 }}>
          ↑ lose these words and your data is gone for good. that's the deal with real encryption.
        </div>

        <Btn amber style={{ width: '100%', justifyContent: 'center', padding: '10px 12px' }}>
          I've saved it — next →
        </Btn>
      </div>
    </div>
  );
}

// ─── Variant C — Side-by-side: phrase + why-it-matters ─────────────────
function OnboardingC() {
  return (
    <div className="wf-card" style={{ width: 640, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ justifyContent: 'space-between', padding: '14px 20px', borderBottom: '1.2px solid var(--ink)' }}>
        <Logo size={14} />
        <div className="wf-row" style={{ gap: 4 }}>
          {[1,2,3,4].map(i => (
            <div key={i} style={{
              width: 26, height: 4,
              background: i <= 3 ? 'var(--ink)' : 'var(--paper-2)',
              border: '1px solid var(--ink)',
            }} />
          ))}
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1.2fr 1fr' }}>
        {/* Left: the phrase */}
        <div style={{ padding: 22, borderRight: '1.2px solid var(--ink)' }}>
          <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 }}>
            Recovery phrase · 12 words
          </div>
          <h2 style={{ fontSize: 18, margin: '0 0 14px', fontWeight: 600 }}>
            Your master key, in words.
          </h2>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '6px 14px' }}>
            {RECOVERY_WORDS.map((w, i) => (
              <div key={i} style={{ display: 'flex', gap: 8, fontFamily: 'var(--mono)', fontSize: 12, padding: '4px 0', borderBottom: '1px dashed rgba(26,26,26,0.15)' }}>
                <span style={{ color: 'var(--ink-4)', width: 16 }}>{String(i + 1).padStart(2, '0')}</span>
                <span>{w}</span>
              </div>
            ))}
          </div>

          <div className="wf-row" style={{ gap: 6, marginTop: 14 }}>
            <Btn alt small>⧉</Btn>
            <Btn alt small>↓ PDF</Btn>
            <Btn alt small>◇ Print</Btn>
            <span className="wf-annot" style={{ marginLeft: 'auto', fontSize: 12 }}>offline storage →</span>
          </div>
        </div>

        {/* Right: why / what happens */}
        <div style={{ padding: 22, background: 'var(--paper-2)' }}>
          <div style={{ fontFamily: 'var(--hand)', fontSize: 18, marginBottom: 10, lineHeight: 1.2 }}>
            Why this matters
          </div>
          {[
            ['We', "can't see it. Ever."],
            ['We', "can't reset it if lost."],
            ['You', 'are the only custodian.'],
            ['This', 'is what zero-knowledge means.'],
          ].map(([a, b], i) => (
            <div key={i} style={{ display: 'flex', gap: 8, fontSize: 12, marginBottom: 8, color: 'var(--ink)' }}>
              <span style={{ fontWeight: 600, width: 28 }}>{a}</span>
              <span style={{ color: 'var(--ink-2)' }}>{b}</span>
            </div>
          ))}
          <div style={{ borderTop: '1px dashed rgba(26,26,26,0.3)', margin: '14px 0' }} />
          <label style={{ display: 'flex', gap: 8, fontSize: 11, color: 'var(--ink-2)', marginBottom: 10 }}>
            <span style={{ width: 13, height: 13, background: 'var(--ink)', display: 'inline-block', flexShrink: 0, position: 'relative' }}>
              <span style={{ position: 'absolute', inset: 2, color: 'var(--paper)', fontSize: 9, lineHeight: 1 }}>✓</span>
            </span>
            Stored offline. Understood.
          </label>
          <Btn amber style={{ width: '100%', justifyContent: 'center' }}>Continue</Btn>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { OnboardingA, OnboardingB, OnboardingC });
