// hifi-tweaks.jsx — Wave 9: persona swap + tweak panel

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "persona": "journalist",
  "density": "comfortable",
  "accent": "amber",
  "region": "Frankfurt",
  "corner": "soft",
  "grain": true,
  "previewTheme": "light"
}/*EDITMODE-END*/;

const PERSONAS = {
  journalist: {
    label: 'Journalist',
    name: 'Isa Marchetti',
    email: 'isa@example.eu',
    avatar: 'IM',
    avatarGrad: 'linear-gradient(135deg, oklch(0.8 0.15 82), oklch(0.6 0.14 50))',
    plan: 'Team · Investigative tier',
    blurb: 'Lead reporter · freelance · 2 editors, 1 legal',
    folders: ['investigations/', 'sources/', 'legal/', 'archive/'],
    file: 'story-draft.md',
    region: 'Frankfurt',
  },
  solo: {
    label: 'Solo founder',
    name: 'Jonas Weber',
    email: 'jonas@example.com',
    avatar: 'JW',
    avatarGrad: 'linear-gradient(135deg, oklch(0.78 0.14 165), oklch(0.55 0.14 200))',
    plan: 'Personal · 200 GB',
    blurb: 'One-person company · tax + product + investor docs',
    folders: ['contracts/', 'financials/', 'product/', 'personal/'],
    file: 'term-sheet-v3.docx',
    region: 'Frankfurt',
  },
  team: {
    label: 'Design team',
    name: 'Studio Kabine',
    email: 'hello@kabine.studio',
    avatar: 'SK',
    avatarGrad: 'linear-gradient(135deg, oklch(0.82 0.14 40), oklch(0.6 0.16 20))',
    plan: 'Team · 12 seats · 2 TB',
    blurb: '12 designers · client work requires confidentiality',
    folders: ['client-a/', 'client-b/', 'assets/', 'internal/'],
    file: 'brand-guidelines-v4.fig',
    region: 'Paris',
  },
  legal: {
    label: 'Law firm',
    name: 'Van Houten & Partners',
    email: 'files@vh-legal.nl',
    avatar: 'VH',
    avatarGrad: 'linear-gradient(135deg, oklch(0.7 0.12 260), oklch(0.5 0.14 280))',
    plan: 'Enterprise · unlimited · audited',
    blurb: '40 lawyers · client privilege is non-negotiable',
    folders: ['matters/', 'discovery/', 'billables/', 'admin/'],
    file: 'matter-2411-brief.pdf',
    region: 'Frankfurt',
  },
};

const ACCENTS = {
  amber:  { base: 'oklch(0.82 0.16 82)', deep: 'oklch(0.52 0.14 66)', bg: 'oklch(0.96 0.05 82)' },
  moss:   { base: 'oklch(0.74 0.13 142)', deep: 'oklch(0.42 0.1 150)', bg: 'oklch(0.95 0.04 142)' },
  ink:    { base: 'oklch(0.6 0.05 250)', deep: 'oklch(0.32 0.04 250)', bg: 'oklch(0.94 0.02 250)' },
  clay:   { base: 'oklch(0.78 0.12 40)', deep: 'oklch(0.48 0.12 30)', bg: 'oklch(0.96 0.04 40)' },
  violet: { base: 'oklch(0.72 0.13 300)', deep: 'oklch(0.42 0.14 295)', bg: 'oklch(0.95 0.04 300)' },
};

// React context for persona
const TweakCtx = React.createContext({ tweaks: TWEAK_DEFAULTS, persona: PERSONAS.journalist });
window.useTweaks = () => React.useContext(TweakCtx);

function TweakProvider({ children }) {
  const [tweaks, setTweaks] = React.useState(TWEAK_DEFAULTS);
  const [active, setActive] = React.useState(false);

  // Listen to host messages
  React.useEffect(() => {
    function onMsg(e) {
      if (!e.data || typeof e.data !== 'object') return;
      if (e.data.type === '__activate_edit_mode') setActive(true);
      if (e.data.type === '__deactivate_edit_mode') setActive(false);
    }
    window.addEventListener('message', onMsg);
    // Only after listener is live:
    window.parent.postMessage({ type: '__edit_mode_available' }, '*');
    return () => window.removeEventListener('message', onMsg);
  }, []);

  // Apply side-effects: CSS vars + body class
  React.useEffect(() => {
    const a = ACCENTS[tweaks.accent] || ACCENTS.amber;
    const root = document.documentElement;
    root.style.setProperty('--amber', a.base);
    root.style.setProperty('--amber-deep', a.deep);
    root.style.setProperty('--amber-bg', a.bg);

    // density
    const dens = tweaks.density === 'compact' ? 0.88 : tweaks.density === 'spacious' ? 1.08 : 1;
    root.style.setProperty('--density', dens);

    // corner radius override
    const cr = tweaks.corner === 'square' ? 0.4 : tweaks.corner === 'round' ? 1.4 : 1;
    root.style.setProperty('--corner-scale', cr);

    // grain on body
    document.body.classList.toggle('bb-grain', !!tweaks.grain);
  }, [tweaks]);

  function update(patch) {
    setTweaks(t => {
      const next = { ...t, ...patch };
      window.parent.postMessage({ type: '__edit_mode_set_keys', edits: patch }, '*');
      return next;
    });
  }

  const persona = PERSONAS[tweaks.persona] || PERSONAS.journalist;
  // Override persona region with tweak region
  const effective = { ...persona, region: tweaks.region || persona.region };

  return (
    <TweakCtx.Provider value={{ tweaks, update, persona: effective }}>
      {children}
      {active && <TweakPanel tweaks={tweaks} update={update} />}
    </TweakCtx.Provider>
  );
}

function TweakPanel({ tweaks, update }) {
  return (
    <div style={{
      position: 'fixed', right: 20, bottom: 20, zIndex: 10000,
      width: 300, background: 'var(--paper)',
      borderRadius: 14, border: '1px solid var(--line-2)',
      boxShadow: '0 24px 60px -20px rgba(0,0,0,0.28), 0 0 0 1px var(--line)',
      fontFamily: 'var(--font-sans)', fontSize: 12.5, color: 'var(--ink)',
      overflow: 'hidden',
    }}>
      <div style={{ padding: '11px 14px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8, background: 'var(--paper-2)' }}>
        <svg width="11" height="12" viewBox="0 0 20 22"><polygon points="10,1 18,6 18,16 10,21 2,16 2,6" fill="var(--amber-deep)" /></svg>
        <div style={{ fontWeight: 600, fontSize: 12 }}>Tweaks</div>
        <div className="t-mono" style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--ink-3)' }}>persona · look</div>
      </div>

      <div style={{ padding: '12px 14px', maxHeight: '70vh', overflow: 'auto' }}>

        <TweakGroup label="Persona">
          <Segments value={tweaks.persona} onChange={v => update({ persona: v })}
            options={[['journalist', 'Reporter'], ['solo', 'Solo'], ['team', 'Studio'], ['legal', 'Legal']]} grid={2} />
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 8, lineHeight: 1.5 }}>
            <strong style={{ color: 'var(--ink)' }}>{(PERSONAS[tweaks.persona] || PERSONAS.journalist).name}</strong>
            <div>{(PERSONAS[tweaks.persona] || PERSONAS.journalist).blurb}</div>
          </div>
        </TweakGroup>

        <TweakGroup label="Accent">
          <div style={{ display: 'flex', gap: 6 }}>
            {Object.entries(ACCENTS).map(([k, a]) => (
              <button key={k} onClick={() => update({ accent: k })} style={{
                flex: 1, height: 32, borderRadius: 8, cursor: 'pointer',
                background: a.base, border: '2px solid',
                borderColor: tweaks.accent === k ? 'var(--ink)' : 'transparent',
                outline: 'none',
              }} title={k} />
            ))}
          </div>
        </TweakGroup>

        <TweakGroup label="Density">
          <Segments value={tweaks.density} onChange={v => update({ density: v })}
            options={[['compact', 'Compact'], ['comfortable', 'Normal'], ['spacious', 'Spacious']]} />
        </TweakGroup>

        <TweakGroup label="Corner radius">
          <Segments value={tweaks.corner} onChange={v => update({ corner: v })}
            options={[['square', 'Crisp'], ['soft', 'Soft'], ['round', 'Round']]} />
        </TweakGroup>

        <TweakGroup label="Region">
          <Segments value={tweaks.region} onChange={v => update({ region: v })}
            options={[['Frankfurt', 'Frankfurt'], ['Paris', 'Paris'], ['Stockholm', 'Stockholm']]} />
        </TweakGroup>

        <TweakGroup label="Paper grain">
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', fontSize: 12 }}>
            <input type="checkbox" checked={tweaks.grain} onChange={e => update({ grain: e.target.checked })} />
            <span style={{ color: 'var(--ink-2)' }}>Subtle paper texture behind canvas</span>
          </label>
        </TweakGroup>

        <TweakGroup label="File preview theme">
          <Segments value={tweaks.previewTheme || 'light'} onChange={v => update({ previewTheme: v })}
            options={[['light', 'Light'], ['dark', 'Dark (lightbox)']]} />
        </TweakGroup>
      </div>

      <div style={{ padding: '8px 14px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', fontSize: 10, color: 'var(--ink-3)', display: 'flex', alignItems: 'center', gap: 6 }}>
        <span>Changes persist · applies across the whole canvas</span>
      </div>
    </div>
  );
}

function TweakGroup({ label, children }) {
  return (
    <div style={{ marginBottom: 14 }}>
      <div className="t-label" style={{ fontSize: 9.5, marginBottom: 6 }}>{label}</div>
      {children}
    </div>
  );
}

function Segments({ value, onChange, options, grid }) {
  return (
    <div style={{
      display: grid ? 'grid' : 'flex',
      gridTemplateColumns: grid ? `repeat(${grid}, 1fr)` : undefined,
      gap: 4,
      padding: 3, borderRadius: 8,
      background: 'var(--paper-2)', border: '1px solid var(--line)',
    }}>
      {options.map(([v, l]) => (
        <button key={v} onClick={() => onChange(v)} style={{
          flex: 1, padding: '5px 8px', fontSize: 11.5, fontWeight: value === v ? 600 : 400,
          color: value === v ? 'var(--paper)' : 'var(--ink-2)',
          background: value === v ? 'var(--ink)' : 'transparent',
          border: 'none', borderRadius: 5, cursor: 'pointer',
          fontFamily: 'inherit',
        }}>{l}</button>
      ))}
    </div>
  );
}

// PersonaBadge — drop anywhere to show current persona info
function PersonaBadge() {
  const { persona } = window.useTweaks();
  return (
    <div style={{
      display: 'inline-flex', alignItems: 'center', gap: 8,
      padding: '6px 10px 6px 6px', borderRadius: 999,
      background: 'var(--paper-2)', border: '1px solid var(--line)',
      fontSize: 11.5,
    }}>
      <div style={{
        width: 22, height: 22, borderRadius: '50%',
        background: persona.avatarGrad,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: 'var(--paper)', fontSize: 9.5, fontWeight: 700,
      }}>{persona.avatar}</div>
      <span style={{ fontWeight: 500 }}>{persona.name}</span>
      <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)' }}>· {persona.plan}</span>
    </div>
  );
}

// Persona overview artboard — showcases the swap
function HiPersonaOverview() {
  const { persona, tweaks } = window.useTweaks();
  return (
    <div style={{ width: 960, padding: 28 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 18, marginBottom: 24 }}>
        <div style={{
          width: 72, height: 72, borderRadius: '50%',
          background: persona.avatarGrad,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          color: 'var(--paper)', fontSize: 26, fontWeight: 700,
          flexShrink: 0,
        }}>{persona.avatar}</div>
        <div style={{ flex: 1 }}>
          <div className="t-label" style={{ fontSize: 10, marginBottom: 4 }}>PERSONA · {(persona.label || '').toUpperCase()}</div>
          <div className="t-display" style={{ fontSize: 32, marginBottom: 4 }}>{persona.name}</div>
          <div style={{ fontSize: 14, color: 'var(--ink-2)' }}>{persona.email}</div>
          <div style={{ fontSize: 13, color: 'var(--ink-3)', marginTop: 6, maxWidth: 480, lineHeight: 1.5 }}>{persona.blurb}</div>
        </div>
        <BBRegionBadge region={persona.region} />
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 14, marginBottom: 24 }}>
        <div style={{ padding: 16, borderRadius: 12, background: 'var(--paper-2)', border: '1px solid var(--line)' }}>
          <div className="t-label" style={{ fontSize: 10, marginBottom: 6 }}>Plan</div>
          <div style={{ fontSize: 14, fontWeight: 600 }}>{persona.plan}</div>
        </div>
        <div style={{ padding: 16, borderRadius: 12, background: 'var(--paper-2)', border: '1px solid var(--line)' }}>
          <div className="t-label" style={{ fontSize: 10, marginBottom: 6 }}>Vault region</div>
          <div style={{ fontSize: 14, fontWeight: 600, fontFamily: 'var(--font-mono)' }}>{persona.region}</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>pinned · will not replicate</div>
        </div>
        <div style={{ padding: 16, borderRadius: 12, background: 'var(--amber-bg)', border: '1px solid oklch(0.88 0.07 88)' }}>
          <div className="t-label" style={{ fontSize: 10, marginBottom: 6, color: 'var(--amber-deep)' }}>Look</div>
          <div style={{ fontSize: 13, fontWeight: 500 }}>Accent <span className="t-mono" style={{ color: 'var(--amber-deep)' }}>{tweaks.accent}</span></div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{tweaks.density} · {tweaks.corner}</div>
        </div>
      </div>

      <div style={{ border: '1px solid var(--line)', borderRadius: 12, overflow: 'hidden' }}>
        <div style={{ padding: '10px 16px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', fontSize: 12, fontWeight: 600 }}>
          Vault — top level
        </div>
        {persona.folders.map((f, i, arr) => (
          <div key={i} style={{
            padding: '11px 16px', display: 'flex', alignItems: 'center', gap: 10,
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
          }}>
            <Ico name="folder" size={13} color="var(--amber-deep)" />
            <span style={{ fontSize: 13, fontFamily: 'var(--font-mono)' }}>{f}</span>
            <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--font-mono)' }}>
              {['142 files', '87 files', '23 files', '1.2k files'][i]}
            </span>
          </div>
        ))}
      </div>

      <div style={{ marginTop: 18, padding: 14, background: 'var(--paper)', border: '1.5px dashed var(--line-2)', borderRadius: 12, display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>
          Toggle <strong style={{ color: 'var(--amber-deep)' }}>Tweaks</strong> in the toolbar to swap persona, accent, density, region and corner shape — every artboard on the canvas updates live.
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { TweakProvider, TweakPanel, PersonaBadge, HiPersonaOverview });
