// hifi-brand-system.jsx — Design system documentation

function SwatchRow({ label, token, hex, varname, textOn }) {
  return (
    <div style={{
      display: 'grid', gridTemplateColumns: '120px 1fr 1fr',
      gap: 12, padding: '10px 14px', alignItems: 'center',
      borderBottom: '1px solid var(--line)',
    }}>
      <div style={{
        height: 40, borderRadius: 6, background: `var(${varname})`,
        border: '1px solid var(--line)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: textOn || 'var(--ink)', fontSize: 11, fontWeight: 500,
      }}>{label}</div>
      <div>
        <div className="t-mono" style={{ fontSize: 11.5, fontWeight: 600 }}>{token}</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{varname}</div>
      </div>
      <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>{hex}</div>
    </div>
  );
}

function DSSection({ title, subtitle, children }) {
  return (
    <div className="bb-card" style={{ padding: 0, flexShrink: 0 }}>
      <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
        <div className="t-h3" style={{ fontSize: 14 }}>{title}</div>
        {subtitle && <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{subtitle}</div>}
      </div>
      <div style={{ padding: 0 }}>{children}</div>
    </div>
  );
}

function HiBrandSystem() {
  const typeSamples = [
    { name: 'Display', cls: 't-display', use: 'Landing heroes · section openers', sample: 'Storage you can\'t read.' },
    { name: 'H1', cls: 't-h1', use: 'Page titles', sample: 'Security overview' },
    { name: 'H2', cls: 't-h2', use: 'Section headers', sample: 'Trusted devices' },
    { name: 'H3', cls: 't-h3', use: 'Card titles', sample: 'Recovery phrase' },
    { name: 'Body', cls: 't-body', use: 'Paragraph copy · longer explanations that wrap across multiple lines', sample: 'Files are encrypted on your device with a key we never see.' },
    { name: 'Small', cls: 't-small', use: 'Secondary copy · metadata', sample: 'Last confirmed 14 Apr · 8 days ago' },
    { name: 'Label', cls: 't-label', use: 'Section eyebrows · mono caps', sample: 'Why Beebeeb' },
    { name: 'Mono', cls: 't-mono', use: 'Code, timestamps, hashes', sample: 'sha256 · 4f9e…a20b' },
  ];

  return (
    <div style={{ width: 1180, display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Cover */}
      <div className="bb-card elevated" style={{ padding: 0, flexShrink: 0, overflow: 'hidden' }}>
        <div style={{ padding: '32px 32px 28px', background: 'var(--paper)', position: 'relative' }}>
          <div className="bb-honeycomb" style={{ position: 'absolute', inset: 0, opacity: 0.4 }} />
          <div style={{ position: 'relative' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 18 }}>
              <BBLogo size={22} />
              <BBChip style={{ marginLeft: 8 }}>v0.1 · April 2026</BBChip>
              <BBChip variant="amber">Internal</BBChip>
            </div>
            <div className="t-display" style={{ fontSize: 44, marginBottom: 8 }}>Design system.</div>
            <div style={{ fontSize: 15, color: 'var(--ink-2)', maxWidth: 600, lineHeight: 1.55 }}>
              Calm. Precise. Honest. The tokens, components, and principles behind every surface of Beebeeb — web, iOS, Android, desktop, marketing.
            </div>
          </div>
        </div>

        {/* Principles strip */}
        <div style={{
          padding: '18px 32px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)',
          display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 22,
        }}>
          {[
            ['01', 'Honest over reassuring', 'If we can\'t recover it, say so.'],
            ['02', 'Mono for what is true', 'Hashes, timestamps, IDs — never body copy.'],
            ['03', 'Amber is a promise', 'One accent. Use it for encryption state, nothing else.'],
            ['04', 'EU tells, not shows', 'Name the city and operator, skip the flag emoji.'],
          ].map(([n, t, d], i) => (
            <div key={i}>
              <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--amber-deep)', fontWeight: 600, marginBottom: 4, letterSpacing: '0.08em' }}>{n}</div>
              <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 2 }}>{t}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', lineHeight: 1.5 }}>{d}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Two-column row: Logo + Color */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1.3fr', gap: 16 }}>
        {/* Logo / mark */}
        <DSSection title="Mark" subtitle="Single honeycomb cell + wordmark. No gradient, no glow, no stroke.">
          <div style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div style={{
              padding: '28px 20px', background: 'var(--paper-2)',
              borderRadius: 'var(--r-2)', textAlign: 'center',
            }}>
              <BBLogo size={28} />
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
              <div style={{ padding: 16, background: 'var(--paper-2)', borderRadius: 'var(--r-2)', textAlign: 'center' }}>
                <BBHex size={32} />
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 10 }}>Glyph only · favicon, social</div>
              </div>
              <div style={{ padding: 16, background: 'var(--ink)', borderRadius: 'var(--r-2)', textAlign: 'center' }}>
                <BBLogo size={18} style={{ color: 'var(--paper)' }} />
                <div className="t-micro" style={{ color: 'oklch(0.7 0.01 80)', marginTop: 10 }}>On ink · reversed</div>
              </div>
            </div>
            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
              <BBChip style={{ fontSize: 9.5 }}>Min clear-space: 1× cell</BBChip>
              <BBChip style={{ fontSize: 9.5 }}>Min size: 14px</BBChip>
              <BBChip style={{ fontSize: 9.5 }}>All-lowercase always</BBChip>
            </div>
          </div>
        </DSSection>

        {/* Color tokens */}
        <DSSection title="Color" subtitle="Warm off-whites, ink-dark neutrals, one amber. No secondary brand color exists by design.">
          <SwatchRow label="" token="paper" hex="oklch(0.985 0.004 85)" varname="--paper" />
          <SwatchRow label="" token="paper-2" hex="oklch(0.968 0.006 85)" varname="--paper-2" />
          <SwatchRow label="" token="line" hex="oklch(0.90 0.008 82)" varname="--line" />
          <SwatchRow label="INK" token="ink" hex="oklch(0.18 0.01 70)" varname="--ink" textOn="var(--amber)" />
          <SwatchRow label="" token="ink-3" hex="oklch(0.52 0.01 78)" varname="--ink-3" textOn="var(--paper)" />
          <SwatchRow label="AMBER" token="amber" hex="oklch(0.82 0.17 84)" varname="--amber" />
          <SwatchRow label="" token="amber-deep" hex="oklch(0.66 0.15 72)" varname="--amber-deep" textOn="var(--paper)" />
          <SwatchRow label="" token="green" hex="oklch(0.72 0.16 155)" varname="--green" />
          <SwatchRow label="" token="red" hex="oklch(0.62 0.21 25)" varname="--red" textOn="var(--paper)" />
        </DSSection>
      </div>

      {/* Typography */}
      <DSSection title="Typography" subtitle="Inter for everything human. JetBrains Mono for everything machine.">
        <div style={{ display: 'grid', gridTemplateColumns: '1.6fr 1fr', gap: 0 }}>
          <div style={{ padding: '4px 0', borderRight: '1px solid var(--line)' }}>
            {typeSamples.map((t, i) => (
              <div key={i} style={{
                padding: '12px 18px',
                borderBottom: i < typeSamples.length - 1 ? '1px solid var(--line)' : 'none',
                display: 'grid', gridTemplateColumns: '90px 1fr', gap: 14, alignItems: 'baseline',
              }}>
                <div>
                  <div className="t-mono" style={{ fontSize: 10.5, fontWeight: 600 }}>{t.name}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-4)' }}>.{t.cls}</div>
                </div>
                <div>
                  <div className={t.cls} style={{ marginBottom: 4 }}>{t.sample}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{t.use}</div>
                </div>
              </div>
            ))}
          </div>
          <div style={{ padding: 20, display: 'flex', flexDirection: 'column', gap: 14 }}>
            <div>
              <div className="t-label" style={{ marginBottom: 6 }}>Inter — sans</div>
              <div style={{ fontFamily: 'var(--font-sans)', fontSize: 28, letterSpacing: '-0.025em', fontWeight: 700 }}>Aa</div>
              <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', marginTop: 4 }}>
                400 · 500 · 600 · 700 · 800
              </div>
            </div>
            <div>
              <div className="t-label" style={{ marginBottom: 6 }}>JetBrains Mono</div>
              <div style={{ fontFamily: 'var(--font-mono)', fontSize: 28, fontWeight: 600 }}>Aa</div>
              <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', marginTop: 4 }}>
                400 · 500 · 600
              </div>
            </div>
            <div style={{
              padding: 12, borderRadius: 'var(--r-2)', background: 'var(--paper-2)',
              border: '1px solid var(--line)', fontSize: 11.5, color: 'var(--ink-2)', lineHeight: 1.55,
            }}>
              <strong>Rule:</strong> if a user can't read it aloud to another human, it's mono.
              Hashes, IDs, timestamps, filenames, sizes — mono. Sentences — sans.
            </div>
          </div>
        </div>
      </DSSection>

      {/* Spacing · radii · shadows */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 16 }}>
        <DSSection title="Spacing" subtitle="4px base">
          <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 8 }}>
            {[['xs', 4], ['sm', 8], ['md', 12], ['lg', 18], ['xl', 24], ['2xl', 36]].map(([t, px], i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div className="t-mono" style={{ fontSize: 11, width: 32, color: 'var(--ink-3)' }}>{t}</div>
                <div style={{ height: 8, background: 'var(--amber)', borderRadius: 2, width: px * 2 }} />
                <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)' }}>{px}px</div>
              </div>
            ))}
          </div>
        </DSSection>
        <DSSection title="Radius" subtitle="Restrained — never > 14px">
          <div style={{ padding: 20, display: 'flex', gap: 10, flexWrap: 'wrap' }}>
            {[['r-1', 4], ['r-2', 6], ['r-3', 10], ['r-4', 14], ['pill', 999]].map(([t, px], i) => (
              <div key={i} style={{
                width: 62, padding: '10px 0', textAlign: 'center',
                borderRadius: px, background: 'var(--paper-2)', border: '1px solid var(--line-2)',
              }}>
                <div className="t-mono" style={{ fontSize: 10.5, fontWeight: 600 }}>{t}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{px === 999 ? '∞' : `${px}px`}</div>
              </div>
            ))}
          </div>
        </DSSection>
        <DSSection title="Elevation" subtitle="Three levels. That's it.">
          <div style={{ padding: 20, display: 'flex', gap: 10 }}>
            {['shadow-1', 'shadow-2', 'shadow-3'].map((t, i) => (
              <div key={i} style={{
                flex: 1, aspectRatio: '1 / 0.8', borderRadius: 'var(--r-2)',
                background: 'var(--paper)', border: '1px solid var(--line)',
                boxShadow: `var(--${t})`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <div className="t-mono" style={{ fontSize: 10.5, fontWeight: 600 }}>{i + 1}</div>
              </div>
            ))}
          </div>
        </DSSection>
      </div>

      {/* Components catalog */}
      <DSSection title="Components" subtitle="The kit. Every screen is built from these.">
        <div style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 20 }}>
          {/* Buttons */}
          <div>
            <div className="t-label" style={{ marginBottom: 10 }}>Buttons</div>
            <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
              <BBBtn variant="amber">Primary action</BBBtn>
              <BBBtn variant="primary">Secondary (ink)</BBBtn>
              <BBBtn>Default</BBBtn>
              <BBBtn variant="ghost">Ghost</BBBtn>
              <BBBtn variant="danger">Danger</BBBtn>
              <BBBtn size="sm">Small</BBBtn>
              <BBBtn size="lg" variant="amber">Large</BBBtn>
              <BBBtn icon={<Ico name="download" size={12} />}>With icon</BBBtn>
            </div>
          </div>
          {/* Chips */}
          <div>
            <div className="t-label" style={{ marginBottom: 10 }}>Chips · badges</div>
            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
              <BBChip>Default</BBChip>
              <BBChip variant="amber"><Ico name="shield" size={10} /> Zero-knowledge</BBChip>
              <BBChip variant="green"><span className="dot" /> EU · Frankfurt</BBChip>
              <BBChip variant="filled">Filled</BBChip>
              <BBRegionBadge region="Amsterdam" />
            </div>
          </div>
          {/* Inputs + toggles */}
          <div>
            <div className="t-label" style={{ marginBottom: 10 }}>Inputs · controls</div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12, alignItems: 'center' }}>
              <div className="bb-input" style={{ gridColumn: 'span 2' }}>
                <Ico name="search" size={13} color="var(--ink-3)" />
                <input placeholder="Search encrypted files…" />
                <BBKbd>⌘K</BBKbd>
              </div>
              <div style={{ display: 'flex', gap: 14, alignItems: 'center' }}>
                <BBCheck on /><BBCheck /><BBToggle on /><BBToggle />
              </div>
            </div>
          </div>
          {/* Iconography */}
          <div>
            <div className="t-label" style={{ marginBottom: 10 }}>Iconography · 1.6 stroke, 24-grid</div>
            <div style={{
              display: 'grid', gridTemplateColumns: 'repeat(12, 1fr)', gap: 8,
              padding: 12, background: 'var(--paper-2)', borderRadius: 'var(--r-2)',
              border: '1px solid var(--line)',
            }}>
              {['folder', 'file', 'image', 'upload', 'download', 'share', 'lock', 'shield', 'key', 'users', 'search', 'plus', 'star', 'trash', 'settings', 'check', 'chevRight', 'chevDown', 'copy', 'eye', 'link', 'clock', 'cloud', 'gallery'].map(n => (
                <div key={n} style={{
                  aspectRatio: '1', display: 'flex', alignItems: 'center', justifyContent: 'center',
                  background: 'var(--paper)', borderRadius: 4, color: 'var(--ink-2)',
                  border: '1px solid var(--line)',
                }}>
                  <Ico name={n} size={14} />
                </div>
              ))}
            </div>
          </div>
        </div>
      </DSSection>

      {/* Voice & copy */}
      <DSSection title="Voice · copy rules" subtitle="How Beebeeb sounds. Every writer reads this before shipping a string.">
        <div style={{ padding: 22, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 22 }}>
          <div>
            <div className="t-label" style={{ color: 'oklch(0.45 0.12 155)', marginBottom: 10 }}>✓ We say</div>
            {[
              ['"We can\'t recover this for you."', 'Honest dead-end framing'],
              ['"Encrypted before it leaves your device."', 'Action + locus in one line'],
              ['"Stored in Frankfurt · Hetzner."', 'Name the city, name the operator'],
              ['"Links self-destruct by design."', 'Feature as promise'],
              ['"14 days free. No card."', 'Blunt, comma-separated'],
            ].map(([q, d], i) => (
              <div key={i} style={{
                padding: '10px 12px', marginBottom: 8,
                background: 'oklch(0.96 0.03 155 / 0.5)', border: '1px solid oklch(0.87 0.08 155)',
                borderRadius: 'var(--r-2)',
              }}>
                <div style={{ fontSize: 13, fontWeight: 500, marginBottom: 2 }}>{q}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{d}</div>
              </div>
            ))}
          </div>
          <div>
            <div className="t-label" style={{ color: 'var(--red)', marginBottom: 10 }}>✗ We don't say</div>
            {[
              ['"Bank-grade security."', 'Meaningless. Which bank? What grade?'],
              ['"Your data is safe with us."', 'Safe how. With us how.'],
              ['"Seamless collaboration."', 'Slop adjective. Cut.'],
              ['"Sovereign cloud 🇪🇺"', 'Emojis weaken the claim'],
              ['"Try our powerful suite"', 'No adjectives that can\'t be measured.'],
            ].map(([q, d], i) => (
              <div key={i} style={{
                padding: '10px 12px', marginBottom: 8,
                background: 'oklch(0.97 0.02 25 / 0.5)', border: '1px solid oklch(0.88 0.05 25)',
                borderRadius: 'var(--r-2)',
              }}>
                <div style={{ fontSize: 13, fontWeight: 500, marginBottom: 2, color: 'var(--ink-2)' }}>{q}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{d}</div>
              </div>
            ))}
          </div>
        </div>
      </DSSection>

      {/* Motion */}
      <DSSection title="Motion" subtitle="Fast, confident, never bouncy. cubic-bezier(0.2, 0.8, 0.3, 1).">
        <div style={{ padding: 22, display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 14 }}>
          {[
            ['Micro · 80ms', 'Row hover, subtle tints'],
            ['Base · 150ms', 'Toggles, checkboxes, dropdowns'],
            ['Transition · 300ms', 'Sheet open, page change, progress fills'],
          ].map(([t, d], i) => (
            <div key={i} style={{
              padding: 14, background: 'var(--paper-2)', border: '1px solid var(--line)',
              borderRadius: 'var(--r-2)',
            }}>
              <div className="t-mono" style={{ fontSize: 11.5, fontWeight: 600, marginBottom: 4 }}>{t}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{d}</div>
            </div>
          ))}
        </div>
      </DSSection>
    </div>
  );
}

Object.assign(window, { HiBrandSystem });
