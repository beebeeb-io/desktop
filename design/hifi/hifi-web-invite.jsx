// hifi-web-invite.jsx — invite flows, welcome tour, desktop install, command palette, full search, shortcuts

function HiAcceptInvite() {
  return (
    <div style={{ width: 520, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: 26, borderBottom: '1px solid var(--line)', textAlign: 'center' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10, marginBottom: 14 }}>
          <div style={{ width: 40, height: 40, borderRadius: 10, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <BBHex size={16} />
          </div>
          <span style={{ fontSize: 22, lineHeight: 1 }}>→</span>
          <div style={{ width: 40, height: 40, borderRadius: 10, background: 'var(--paper-2)', border: '1px solid var(--line-2)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 13, fontWeight: 700 }}>P</div>
        </div>
        <div className="t-h3" style={{ fontWeight: 600 }}>Marc invited you to <span style={{ color: 'var(--amber-deep)' }}>Acme Studio</span></div>
        <div className="t-body" style={{ fontSize: 13, color: 'var(--ink-2)', marginTop: 6 }}>You'll get access to 3 shared folders · 4 members already here</div>
      </div>
      <div style={{ padding: 24 }}>
        <div style={{ padding: 14, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)', marginBottom: 16 }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Shared with you</div>
          {['Tirana-story', 'Legal-review', 'Archive 2025'].map((f, i) => (
            <div key={f} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '5px 0' }}>
              <Ico name="folder" size={13} color="var(--amber-deep)" />
              <span style={{ fontSize: 12.5 }}>{f}</span>
              <span className="t-micro" style={{ marginLeft: 'auto', color: 'var(--ink-3)' }}>{i === 0 ? 'Can edit' : i === 1 ? 'Can view' : 'Can edit'}</span>
            </div>
          ))}
        </div>
        <div className="t-label" style={{ marginBottom: 8 }}>Sign in as</div>
        <div style={{ padding: '10px 12px', borderRadius: 8, border: '1px solid var(--amber-deep)', background: 'var(--amber-bg)', display: 'flex', alignItems: 'center', gap: 10, fontSize: 12.5 }}>
          <div style={{ width: 26, height: 26, borderRadius: '50%', background: 'var(--ink)', color: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, fontWeight: 700 }}>I</div>
          <div>
            <div style={{ fontWeight: 600 }}>isa@example.eu</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Already signed in · vault encrypted</div>
          </div>
          <BBChip variant="green" style={{ marginLeft: 'auto' }}>Zero-knowledge</BBChip>
        </div>
        <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center', marginTop: 16 }}>Accept · join Panorama</BBBtn>
        <div style={{ textAlign: 'center', marginTop: 10 }}>
          <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Or <a style={{ color: 'var(--amber-deep)' }}>create a new Beebeeb account</a></span>
        </div>
      </div>
    </div>
  );
}

function HiWelcomeTour() {
  const steps = [
    { t: 'Your recovery phrase', d: 'This is the only key to your vault. We saved it encrypted — you should print a copy.', done: true },
    { t: 'Add a second device', d: 'Install the desktop app or iOS app so you can work anywhere.', done: true },
    { t: 'Upload your first file', d: 'Drag one in, or drop a folder. Encryption happens on your laptop.', done: false, active: true },
    { t: 'Invite a person', d: 'Keys exchange happens between your devices, not through our servers.', done: false },
    { t: 'Turn on passkey', d: 'Faster sign-in with your device\'s biometrics.', done: false },
  ];
  return (
    <div style={{ width: 680, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '18px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <BBHex size={14} />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Welcome, Isa</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>2 of 5 done</BBChip>
      </div>
      <div style={{ padding: '8px 0' }}>
        {steps.map((s, i) => (
          <div key={i} style={{ padding: '12px 22px', display: 'flex', alignItems: 'flex-start', gap: 14, background: s.active ? 'var(--amber-bg)' : 'transparent', borderLeft: s.active ? '3px solid var(--amber-deep)' : '3px solid transparent' }}>
            <div style={{ width: 22, height: 22, borderRadius: '50%', background: s.done ? 'oklch(0.72 0.16 155)' : s.active ? 'var(--amber-deep)' : 'var(--paper-2)', color: s.done || s.active ? 'var(--paper)' : 'var(--ink-3)', border: s.done || s.active ? 'none' : '1px solid var(--line-2)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, fontWeight: 700, flexShrink: 0, marginTop: 1 }}>{s.done ? '✓' : i + 1}</div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13.5, fontWeight: 600, color: s.done ? 'var(--ink-3)' : 'var(--ink)', textDecoration: s.done ? 'line-through' : 'none' }}>{s.t}</div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-2)', marginTop: 2, lineHeight: 1.5 }}>{s.d}</div>
            </div>
            {s.active && <BBBtn size="sm" variant="amber">Continue</BBBtn>}
          </div>
        ))}
      </div>
      <div style={{ padding: '14px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Skip for now · you can find this in Settings anytime</span>
        <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }}>Dismiss tour</BBBtn>
      </div>
    </div>
  );
}

function HiInstallDesktop() {
  return (
    <div style={{ width: 720, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="download" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Get the desktop app</div>
      </div>
      <div style={{ padding: 26, textAlign: 'center' }}>
        <div style={{ fontSize: 20, fontWeight: 600, letterSpacing: -0.2 }}>Your files, in Finder.</div>
        <div className="t-body" style={{ fontSize: 13.5, color: 'var(--ink-2)', marginTop: 6 }}>Encrypts on your machine before it leaves. Works offline.</div>
        <div style={{ marginTop: 22, display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12 }}>
          {[
            { os: 'macOS', sub: 'Universal · 12.0+', size: '94 MB' },
            { os: 'Windows', sub: 'x64 · 10, 11', size: '112 MB' },
            { os: 'Linux', sub: 'AppImage · deb · rpm', size: '98 MB' },
          ].map((p, i) => (
            <div key={p.os} style={{ padding: 18, borderRadius: 10, background: i === 0 ? 'var(--amber-bg)' : 'var(--paper-2)', border: `1px solid ${i === 0 ? 'var(--amber-deep)' : 'var(--line-2)'}`, textAlign: 'left' }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>{p.os}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4 }}>{p.sub}</div>
              <BBBtn size="sm" variant={i === 0 ? 'amber' : 'default'} style={{ marginTop: 12, width: '100%', justifyContent: 'center' }}><Ico name="download" size={11} /> Download</BBBtn>
              <div className="t-mono t-micro" style={{ color: 'var(--ink-3)', marginTop: 8 }}>{p.size} · SHA-256 verified</div>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 22, padding: 14, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)', textAlign: 'left' }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Prefer terminal?</div>
          <div className="t-mono" style={{ fontSize: 12, color: 'var(--ink-2)', padding: '8px 12px', background: 'var(--ink)', color: 'var(--paper)', borderRadius: 6 }}>curl -fsSL https://beebeeb.io/install.sh | sh</div>
        </div>
      </div>
    </div>
  );
}

function HiCommandPalette() {
  const groups = [
    { l: 'Jump to', items: [
      { i: 'folder', t: 'Tirana-story', s: 'Folder · 24 items', shortcut: '' },
      { i: 'folder', t: 'Legal-review', s: 'Folder · 8 items', shortcut: '' },
      { i: 'file', t: 'minutes-march.pdf', s: 'Tirana-story · 2.4 MB', shortcut: '' },
    ]},
    { l: 'Actions', items: [
      { i: 'upload', t: 'Upload files…', s: '', shortcut: 'U' },
      { i: 'folder', t: 'New folder', s: '', shortcut: '⇧ N' },
      { i: 'share', t: 'Create share link', s: 'Requires selection', shortcut: 'S' },
      { i: 'users', t: 'Invite to workspace…', s: '', shortcut: '' },
    ]},
    { l: 'Settings', items: [
      { i: 'shield', t: 'Security center', s: '', shortcut: 'G S' },
      { i: 'key', t: 'View recovery phrase', s: 'Requires re-auth', shortcut: '' },
    ]},
  ];
  return (
    <div style={{ width: 640, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-3)' }}>
      <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="search" size={13} color="var(--ink-3)" />
        <input defaultValue="tir" placeholder="Type to search or run a command…" style={{ flex: 1, border: 'none', outline: 'none', fontSize: 14, background: 'transparent' }} autoFocus />
        <span className="t-mono t-micro" style={{ padding: '2px 6px', borderRadius: 4, background: 'var(--paper-2)', border: '1px solid var(--line)', color: 'var(--ink-3)' }}>esc</span>
      </div>
      <div style={{ maxHeight: 380, overflow: 'auto', padding: '6px 0' }}>
        {groups.map(g => (
          <div key={g.l}>
            <div className="t-label" style={{ padding: '10px 18px 4px' }}>{g.l}</div>
            {g.items.map((it, i) => (
              <div key={i} style={{ padding: '9px 18px', display: 'flex', alignItems: 'center', gap: 12, background: g.l === 'Jump to' && i === 0 ? 'var(--amber-bg)' : 'transparent', borderLeft: g.l === 'Jump to' && i === 0 ? '2px solid var(--amber-deep)' : '2px solid transparent' }}>
                <Ico name={it.i} size={13} color={g.l === 'Jump to' && i === 0 ? 'var(--amber-deep)' : 'var(--ink-2)'} />
                <span style={{ fontSize: 13 }}>{it.t}</span>
                {it.s && <span className="t-micro" style={{ color: 'var(--ink-3)' }}>· {it.s}</span>}
                {it.shortcut && <span className="t-mono t-micro" style={{ marginLeft: 'auto', padding: '1px 6px', borderRadius: 4, background: 'var(--paper-2)', border: '1px solid var(--line)', color: 'var(--ink-3)' }}>{it.shortcut}</span>}
                {g.l === 'Jump to' && i === 0 && <span className="t-mono t-micro" style={{ marginLeft: 'auto', padding: '1px 6px', borderRadius: 4, background: 'var(--ink)', color: 'var(--paper)' }}>↵</span>}
              </div>
            ))}
          </div>
        ))}
      </div>
      <div style={{ padding: '10px 18px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 16, fontSize: 11, color: 'var(--ink-3)' }}>
        <span><span className="t-mono" style={{ padding: '1px 5px', background: 'var(--paper)', border: '1px solid var(--line)', borderRadius: 3 }}>↑↓</span> navigate</span>
        <span><span className="t-mono" style={{ padding: '1px 5px', background: 'var(--paper)', border: '1px solid var(--line)', borderRadius: 3 }}>↵</span> open</span>
        <span style={{ marginLeft: 'auto' }}>Search runs locally · encrypted filename index</span>
      </div>
    </div>
  );
}

function HiSearchFull() {
  return (
    <div style={{ width: 1160, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="search" size={13} color="var(--ink-2)" />
        <input defaultValue="tirana" style={{ flex: 1, border: 'none', outline: 'none', fontSize: 15 }} />
        <BBChip>24 results</BBChip>
        <BBChip variant="amber">0.14 s · local</BBChip>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', minHeight: 520 }}>
        <div style={{ borderRight: '1px solid var(--line)', padding: 16, background: 'var(--paper-2)' }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Filters</div>
          <div className="t-label" style={{ marginTop: 12, marginBottom: 6, fontSize: 9.5 }}>Type</div>
          {[['All', 24, true], ['Folders', 3, false], ['PDFs', 8, false], ['Images', 9, false], ['Audio', 3, false], ['Docs', 1, false]].map(([l, n, sel]) => (
            <label key={l} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 0', fontSize: 12.5, color: sel ? 'var(--amber-deep)' : 'var(--ink-2)', fontWeight: sel ? 600 : 400 }}>
              <span className={`bb-check ${sel ? 'on' : ''}`} />{l}<span style={{ marginLeft: 'auto', color: 'var(--ink-3)' }}>{n}</span>
            </label>
          ))}
          <div className="t-label" style={{ marginTop: 16, marginBottom: 6, fontSize: 9.5 }}>Where</div>
          {['Anywhere', 'Tirana-story', 'Legal-review', 'Shared with me'].map((l, i) => (
            <label key={l} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 0', fontSize: 12.5, color: i === 0 ? 'var(--amber-deep)' : 'var(--ink-2)', fontWeight: i === 0 ? 600 : 400 }}>
              <span style={{ width: 10, height: 10, borderRadius: '50%', border: '1.5px solid var(--line-2)', background: i === 0 ? 'var(--amber-deep)' : 'transparent' }} />{l}
            </label>
          ))}
          <div className="t-label" style={{ marginTop: 16, marginBottom: 6, fontSize: 9.5 }}>Modified</div>
          {['Any time', 'Past 7 days', 'Past 30 days', 'This year'].map((l, i) => (
            <label key={l} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 0', fontSize: 12.5 }}>
              <span style={{ width: 10, height: 10, borderRadius: '50%', border: '1.5px solid var(--line-2)', background: i === 0 ? 'var(--amber-deep)' : 'transparent' }} />{l}
            </label>
          ))}
        </div>
        <div style={{ padding: '12px 22px' }}>
          {[
            { n: 'minutes-march.pdf', p: 'Tirana-story / evidence', sz: '2.4 MB', mod: '2 days ago', hi: 'tirana' },
            { n: 'tirana-interview-03.wav', p: 'Tirana-story / raw', sz: '142 MB', mod: '3 days ago', hi: 'tirana' },
            { n: 'Tirana-map-annotated.png', p: 'Tirana-story / source', sz: '6.1 MB', mod: '4 days ago', hi: 'Tirana' },
            { n: 'Tirana-story', p: 'root', sz: '24 items · 412 MB', mod: 'updated 2h ago', hi: 'Tirana', folder: true },
            { n: 'court-records-tirana.pdf', p: 'Legal-review', sz: '8.7 MB', mod: '1 week ago', hi: 'tirana' },
          ].map((r, i) => (
            <div key={i} style={{ padding: '12px 0', borderBottom: i < 4 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 14 }}>
              <div style={{ width: 34, height: 34, borderRadius: 8, background: r.folder ? 'var(--amber-bg)' : 'var(--paper-2)', border: '1px solid var(--line-2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Ico name={r.folder ? 'folder' : 'file'} size={14} color={r.folder ? 'var(--amber-deep)' : 'var(--ink-2)'} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13.5, fontWeight: 500 }} dangerouslySetInnerHTML={{__html: r.n.replace(new RegExp(`(${r.hi})`, 'gi'), '<mark style="background:var(--amber-bg);color:var(--ink);padding:0 2px;border-radius:2px">$1</mark>')}} />
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{r.p}</div>
              </div>
              <span className="t-mono t-micro" style={{ width: 100, color: 'var(--ink-3)' }}>{r.sz}</span>
              <span className="t-micro" style={{ width: 120, color: 'var(--ink-3)' }}>{r.mod}</span>
            </div>
          ))}
          <div style={{ marginTop: 18, padding: 14, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)', fontSize: 12, color: 'var(--ink-2)', display: 'flex', alignItems: 'center', gap: 10 }}>
            <Ico name="shield" size={13} color="var(--ink-3)" />
            <span>Filename search is live. <strong>Content search</strong> requires a locally-built index — <a style={{ color: 'var(--amber-deep)' }}>enable on this device</a>. The server never sees your words.</span>
          </div>
        </div>
      </div>
    </div>
  );
}

function HiShortcuts() {
  const groups = [
    { l: 'Navigation', items: [['⌘ K', 'Open command palette'], ['/', 'Search'], ['G then D', 'Drive'], ['G then P', 'Photos'], ['G then S', 'Security'], ['G then T', 'Team'], ['?', 'Shortcuts (this)']]},
    { l: 'File actions', items: [['U', 'Upload'], ['⇧ N', 'New folder'], ['Space', 'Quick preview'], ['↵', 'Open'], ['⌘ D', 'Download'], ['⌘ ⌫', 'Move to trash'], ['F2', 'Rename']]},
    { l: 'Selection', items: [['⌘ A', 'Select all'], ['⇧ ↑↓', 'Extend selection'], ['Esc', 'Clear selection']]},
    { l: 'Sharing', items: [['S', 'Create share link'], ['⇧ S', 'Share with person'], ['⌘ I', 'Show details panel']]},
  ];
  return (
    <div style={{ width: 720, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-3)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ fontSize: 14, fontWeight: 600 }}>Keyboard shortcuts</div>
        <BBChip style={{ marginLeft: 'auto' }}>Press <span className="t-mono" style={{ marginLeft: 4 }}>?</span> anywhere</BBChip>
      </div>
      <div style={{ padding: 22, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 22 }}>
        {groups.map(g => (
          <div key={g.l}>
            <div className="t-label" style={{ marginBottom: 10 }}>{g.l}</div>
            {g.items.map(([k, d], i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', padding: '6px 0', borderBottom: i < g.items.length - 1 ? '1px solid var(--line)' : 'none' }}>
                <span style={{ fontSize: 12.5 }}>{d}</span>
                <span className="t-mono" style={{ marginLeft: 'auto', fontSize: 11, padding: '2px 8px', borderRadius: 4, background: 'var(--paper-2)', border: '1px solid var(--line-2)' }}>{k}</span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

Object.assign(window, { HiAcceptInvite, HiWelcomeTour, HiInstallDesktop, HiCommandPalette, HiSearchFull, HiShortcuts });
