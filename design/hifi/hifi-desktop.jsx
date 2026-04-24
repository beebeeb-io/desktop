// hifi-desktop.jsx — Wave 7: desktop sync client (preferences, selective sync, conflict, first-run)

// Reusable macOS window chrome (self-contained — doesn't depend on starter)
function DesktopWin({ title = 'Beebeeb', width = 820, height = 560, children, toolbar, platform = 'mac' }) {
  return (
    <div style={{
      width, height, borderRadius: platform === 'mac' ? 12 : 6,
      background: 'var(--paper)', overflow: 'hidden',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.28), 0 0 0 1px var(--line-2)',
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 38, borderBottom: '1px solid var(--line)',
        background: 'var(--paper-2)',
        display: 'flex', alignItems: 'center', padding: '0 14px', gap: 10,
        position: 'relative',
      }}>
        {platform === 'mac' ? (
          <div style={{ display: 'flex', gap: 7 }}>
            {['#ff5f57', '#febc2e', '#28c840'].map((c, i) => (
              <div key={i} style={{ width: 12, height: 12, borderRadius: '50%', background: c }} />
            ))}
          </div>
        ) : (
          <div style={{ display: 'flex', gap: 2, marginLeft: 'auto', order: 2 }}>
            {['—', '□', '×'].map((c, i) => (
              <div key={i} style={{ padding: '4px 10px', fontSize: 11, color: 'var(--ink-3)' }}>{c}</div>
            ))}
          </div>
        )}
        <div style={{
          position: 'absolute', left: 0, right: 0, textAlign: 'center',
          fontSize: 12, fontWeight: 500, color: 'var(--ink-2)',
          pointerEvents: 'none',
        }}>{title}</div>
      </div>
      {toolbar}
      <div style={{ flex: 1, overflow: 'hidden' }}>{children}</div>
    </div>
  );
}

// ─── Preferences — General/Account tabs ─────────────────────────────────────
function HiDesktopPrefs() {
  const tabs = ['General', 'Account', 'Sync', 'Bandwidth', 'Security', 'Advanced'];
  return (
    <DesktopWin title="Beebeeb Preferences" width={800} height={600}>
      <div style={{ display: 'flex', height: '100%' }}>
        <div style={{ width: 180, background: 'var(--paper-2)', borderRight: '1px solid var(--line)', padding: 10 }}>
          {tabs.map((t, i) => (
            <div key={i} style={{
              padding: '7px 10px', borderRadius: 6, marginBottom: 2,
              fontSize: 12.5, fontWeight: i === 0 ? 600 : 400,
              background: i === 0 ? 'var(--paper)' : 'transparent',
              border: i === 0 ? '1px solid var(--line)' : '1px solid transparent',
              color: i === 0 ? 'var(--ink)' : 'var(--ink-2)',
            }}>{t}</div>
          ))}
        </div>
        <div style={{ flex: 1, overflow: 'auto', padding: '18px 22px' }}>
          <div className="t-h2" style={{ fontSize: 17, marginBottom: 4 }}>General</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 18 }}>Launch behaviour &amp; system integration</div>

          {[
            { l: 'Launch at login', toggle: true, hint: 'Starts quietly in the menu bar' },
            { l: 'Show in Finder sidebar', toggle: true },
            { l: 'Enable Spotlight search inside vault', toggle: false, hint: 'Index is encrypted and stored locally only' },
            { l: 'Notifications', v: 'Security events only', chev: true },
            { l: 'Locked behaviour', v: 'Pause sync', chev: true, hint: 'What to do when the vault is locked' },
          ].map((r, i, arr) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '1fr 120px',
              padding: '12px 0', alignItems: 'center',
              borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
            }}>
              <div>
                <div style={{ fontSize: 13, fontWeight: 500 }}>{r.l}</div>
                {r.hint && <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{r.hint}</div>}
              </div>
              <div style={{ textAlign: 'right' }}>
                {r.toggle !== undefined && <BBToggle on={r.toggle} />}
                {r.v && <span className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>{r.v} ▾</span>}
              </div>
            </div>
          ))}

          <div style={{ marginTop: 22, padding: 12, background: 'var(--paper-2)', borderRadius: 8, border: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{ width: 28, height: 28, borderRadius: 6, background: 'var(--amber-bg)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Ico name="shield" size={13} color="var(--amber-deep)" />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12.5, fontWeight: 500 }}>Everything below is fully encrypted end-to-end</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>Settings sync in a separate encrypted envelope. Beebeeb cannot read them.</div>
            </div>
          </div>
        </div>
      </div>
    </DesktopWin>
  );
}

// ─── Selective sync — tree view ─────────────────────────────────────
function HiSelectiveSync() {
  const tree = [
    { lvl: 0, n: 'Vault', items: 2834, sz: '23.4 GB', state: 'some', open: true },
    { lvl: 1, n: 'investigations/', items: 847, sz: '12.1 GB', state: 'all', open: true },
    { lvl: 2, n: 'ledger-gap/', items: 23, sz: '142 MB', state: 'all' },
    { lvl: 2, n: 'source-docs/', items: 128, sz: '2.4 GB', state: 'all' },
    { lvl: 2, n: 'interviews-raw/', items: 47, sz: '9.4 GB', state: 'none', onlyOnline: true },
    { lvl: 1, n: 'photos/', items: 1247, sz: '3.4 GB', state: 'all', open: false },
    { lvl: 1, n: 'archive/', items: 412, sz: '6.8 GB', state: 'none', onlyOnline: true },
    { lvl: 1, n: 'team-shared/', items: 88, sz: '412 MB', state: 'all' },
    { lvl: 1, n: 'sketches/', items: 203, sz: '640 MB', state: 'all' },
  ];
  return (
    <DesktopWin title="Beebeeb — Selective Sync" width={780} height={600}>
      <div style={{ padding: '16px 22px', height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div className="t-h2" style={{ fontSize: 17 }}>What to keep on this Mac</div>
        <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2, marginBottom: 14 }}>
          Uncheck folders to keep them online-only. They appear as placeholders until opened.
        </div>

        <div style={{ flex: 1, border: '1px solid var(--line)', borderRadius: 8, overflow: 'auto', background: 'var(--paper)' }}>
          {tree.map((n, i) => {
            const state = n.state;
            const check = state === 'all' ? '✓' : state === 'some' ? '−' : '';
            return (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '24px 20px 14px 1fr 80px 60px',
                alignItems: 'center', gap: 6,
                padding: '7px 10px',
                paddingLeft: 10 + n.lvl * 18,
                borderBottom: i < tree.length - 1 ? '1px solid var(--line)' : 'none',
                background: n.state === 'none' ? 'var(--paper-2)' : 'var(--paper)',
                opacity: n.state === 'none' ? 0.7 : 1,
              }}>
                <div style={{
                  width: 15, height: 15, borderRadius: 3,
                  border: '1.5px solid',
                  borderColor: state === 'none' ? 'var(--line-2)' : 'var(--ink)',
                  background: state === 'none' ? 'var(--paper)' : 'var(--ink)',
                  color: 'var(--paper)', fontSize: 10, fontWeight: 700,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>{check}</div>
                <span style={{ fontSize: 10, color: 'var(--ink-3)' }}>{n.open !== undefined ? (n.open ? '▾' : '▸') : ''}</span>
                <Ico name="folder" size={11} color={state === 'none' ? 'var(--ink-4)' : 'var(--amber-deep)'} />
                <span style={{ fontSize: 12.5, fontWeight: n.lvl === 0 ? 600 : 400, display: 'flex', alignItems: 'center', gap: 8 }}>
                  {n.n}
                  {n.onlyOnline && <BBChip style={{ fontSize: 9, padding: '1px 6px' }}>online-only</BBChip>}
                </span>
                <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', textAlign: 'right' }}>{n.sz}</span>
                <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)', textAlign: 'right' }}>{n.items}</span>
              </div>
            );
          })}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', marginTop: 14, gap: 14 }}>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>
            On disk: <strong style={{ color: 'var(--ink)', fontFamily: 'var(--font-mono)' }}>19.1 GB</strong> · Online-only: <strong style={{ color: 'var(--ink)', fontFamily: 'var(--font-mono)' }}>16.2 GB</strong>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <BBBtn size="sm" variant="ghost">Cancel</BBBtn>
            <BBBtn size="sm" variant="primary">Apply (re-sync 3 folders)</BBBtn>
          </div>
        </div>
      </div>
    </DesktopWin>
  );
}

// ─── Conflict resolver ─────────────────────────────────────
function HiConflict() {
  return (
    <DesktopWin title="Beebeeb — Sync conflict" width={800} height={580}>
      <div style={{ padding: '18px 24px', height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 32, height: 32, borderRadius: 8, background: 'oklch(0.95 0.06 55)', border: '1px solid oklch(0.85 0.1 55)', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <svg width="14" height="14" viewBox="0 0 24 24"><path d="M12 2 L22 20 L2 20 Z" stroke="oklch(0.45 0.15 55)" strokeWidth="1.8" fill="none" /><line x1="12" y1="9" x2="12" y2="14" stroke="oklch(0.45 0.15 55)" strokeWidth="1.8" /><circle cx="12" cy="17" r="0.9" fill="oklch(0.45 0.15 55)" /></svg>
          </div>
          <div style={{ flex: 1 }}>
            <div className="t-h2" style={{ fontSize: 16 }}>This file changed on two devices</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>
              <strong style={{ color: 'var(--ink)', fontFamily: 'var(--font-mono)' }}>story-draft.md</strong> · investigations/ledger-gap/
            </div>
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14, flex: 1 }}>
          {[
            {
              who: 'This Mac', when: '14:22 · 2 min ago', sz: '14.2 KB',
              who2: 'Isa · MacBook Pro',
              lines: ['+ added a fifth witness quote', '+ two cross-references in footnotes', '− removed the unverified figure from §3', '48 words changed'],
              picked: true,
            },
            {
              who: 'iPhone', when: '14:08 · 16 min ago', sz: '13.8 KB',
              who2: 'Isa · iPhone 15 Pro',
              lines: ['+ editor comment resolved', '+ typo fixes throughout', '− struck paragraph about side source', '31 words changed'],
              picked: false,
            },
          ].map((v, i) => (
            <div key={i} style={{
              border: '1.5px solid',
              borderColor: v.picked ? 'var(--amber-deep)' : 'var(--line)',
              borderRadius: 10, padding: 14,
              background: v.picked ? 'var(--amber-bg)' : 'var(--paper)',
              display: 'flex', flexDirection: 'column',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>{v.who}</div>
                {v.picked && <BBChip variant="filled" style={{ fontSize: 9.5, padding: '1px 7px' }}>Keeping</BBChip>}
                <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginLeft: 'auto' }}>{v.sz}</span>
              </div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginBottom: 12 }}>{v.when} · {v.who2}</div>
              <div style={{ flex: 1, background: 'var(--paper)', border: '1px solid var(--line)', borderRadius: 6, padding: 10 }}>
                {v.lines.map((l, li) => (
                  <div key={li} className="t-mono" style={{ fontSize: 10.5, padding: '2px 0', color: l.startsWith('+') ? 'oklch(0.45 0.12 155)' : l.startsWith('−') ? 'var(--red)' : 'var(--ink-3)' }}>{l}</div>
                ))}
              </div>
              <BBBtn size="sm" variant={v.picked ? 'primary' : 'ghost'} style={{ marginTop: 10, justifyContent: 'center' }}>
                {v.picked ? 'Selected' : 'Use this version'}
              </BBBtn>
            </div>
          ))}
        </div>

        <div style={{ display: 'flex', marginTop: 14, alignItems: 'center', gap: 12 }}>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Loser version is kept as <span className="t-mono">story-draft (iPhone, 14:08).md</span> — never silently dropped.</div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <BBBtn size="sm" variant="ghost">Keep both</BBBtn>
            <BBBtn size="sm" variant="primary">Resolve conflict</BBBtn>
          </div>
        </div>
      </div>
    </DesktopWin>
  );
}

// ─── First-run setup ─────────────────────────────────────
function HiFirstRun() {
  return (
    <DesktopWin title="Welcome to Beebeeb" width={720} height={540}>
      <div style={{ height: '100%', display: 'grid', gridTemplateColumns: '260px 1fr' }}>
        <div style={{
          background: 'linear-gradient(160deg, var(--amber-bg), var(--paper-2))',
          borderRight: '1px solid var(--line)',
          padding: '28px 22px', display: 'flex', flexDirection: 'column',
        }}>
          <BBLogo size={17} />
          <div className="t-display" style={{ fontSize: 22, marginTop: 28, lineHeight: 1.15 }}>
            Welcome to your vault.
          </div>
          <div className="t-micro" style={{ color: 'var(--ink-2)', marginTop: 10, lineHeight: 1.6, maxWidth: 200 }}>
            Four quick steps. No data leaves this computer without encryption.
          </div>

          <div style={{ marginTop: 32, display: 'flex', flexDirection: 'column', gap: 14 }}>
            {[
              ['1', 'Unlock with passphrase', true],
              ['2', 'Pick your vault folder', true],
              ['3', 'Selective sync', false, true],
              ['4', 'Finder integration', false],
            ].map(([n, l, done, current], i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  width: 22, height: 22, borderRadius: '50%',
                  background: done ? 'var(--ink)' : current ? 'var(--amber-deep)' : 'var(--paper)',
                  color: (done || current) ? 'var(--paper)' : 'var(--ink-3)',
                  border: (done || current) ? 'none' : '1.5px solid var(--line-2)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 11, fontWeight: 600,
                }}>
                  {done ? '✓' : n}
                </div>
                <span style={{ fontSize: 12.5, fontWeight: current ? 600 : 400, color: current ? 'var(--ink)' : done ? 'var(--ink-2)' : 'var(--ink-3)' }}>{l}</span>
              </div>
            ))}
          </div>

          <div style={{ marginTop: 'auto' }}>
            <BBRegionBadge region="Frankfurt" />
          </div>
        </div>

        <div style={{ padding: '32px 36px', display: 'flex', flexDirection: 'column', height: '100%' }}>
          <div className="t-label" style={{ fontSize: 10 }}>Step 3 of 4</div>
          <div className="t-h2" style={{ fontSize: 22, marginTop: 4 }}>Pick what lives on this Mac</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 6, lineHeight: 1.6, maxWidth: 400 }}>
            Keep everything, or only what you need. Online-only folders appear as placeholders — open them to pull down on demand.
          </div>

          <div style={{ marginTop: 22, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, flex: 1 }}>
            {[
              { t: 'Everything', d: '23.4 GB · fastest but more disk', selected: false },
              { t: 'Smart', d: 'recent + starred + shared · 6.2 GB', selected: true, rec: true },
              { t: 'Custom', d: 'I\'ll pick folders', selected: false },
              { t: 'Online-only', d: 'placeholders only · 0 GB on disk', selected: false },
            ].map((o, i) => (
              <div key={i} style={{
                padding: 14, borderRadius: 10,
                border: '1.5px solid', borderColor: o.selected ? 'var(--amber-deep)' : 'var(--line)',
                background: o.selected ? 'var(--amber-bg)' : 'var(--paper)',
                position: 'relative',
              }}>
                {o.rec && <div style={{ position: 'absolute', top: -8, right: 10, fontSize: 9, padding: '2px 7px', background: 'var(--amber-deep)', color: 'var(--paper)', borderRadius: 6, fontWeight: 600, letterSpacing: 0.3 }}>RECOMMENDED</div>}
                <div style={{ fontSize: 13.5, fontWeight: 600 }}>{o.t}</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4, lineHeight: 1.4 }}>{o.d}</div>
              </div>
            ))}
          </div>

          <div style={{ display: 'flex', alignItems: 'center', marginTop: 20, gap: 12 }}>
            <BBBtn size="sm" variant="ghost">← Back</BBBtn>
            <div className="t-micro" style={{ color: 'var(--ink-4)', marginLeft: 8 }}>You can change this any time in Preferences.</div>
            <div style={{ marginLeft: 'auto' }}>
              <BBBtn size="sm" variant="primary">Continue</BBBtn>
            </div>
          </div>
        </div>
      </div>
    </DesktopWin>
  );
}

// ─── Windows File Explorer with Beebeeb integration ─────────────────────────────────────
function HiWindowsExplorer() {
  const files = [
    { n: 'ledger-gap', t: 'folder', state: 'synced', sz: '142 MB', d: 'Today 14:22' },
    { n: 'source-docs', t: 'folder', state: 'synced', sz: '2.4 GB', d: 'Today 09:41' },
    { n: 'interviews-raw', t: 'folder', state: 'online', sz: '9.4 GB', d: '3 days ago' },
    { n: 'story-draft.md', t: 'doc', state: 'syncing', sz: '14.2 KB', d: 'Just now', prog: 72 },
    { n: 'fact-check-notes.pdf', t: 'pdf', state: 'synced', sz: '2.4 MB', d: 'Yesterday' },
    { n: 'leak-packet.pdf', t: 'pdf', state: 'locked', sz: '1.1 MB', d: '1 week ago' },
    { n: 'interview-03.m4a', t: 'audio', state: 'online', sz: '48 MB', d: '2 weeks ago' },
  ];
  const stateIcon = (s, prog) => {
    if (s === 'syncing') return <div style={{ width: 14, height: 14, borderRadius: '50%', border: '1.5px solid var(--line-2)', borderTopColor: 'var(--amber-deep)', animation: 'spin 1s linear infinite' }} />;
    if (s === 'synced') return <div style={{ width: 14, height: 14, borderRadius: '50%', background: 'oklch(0.65 0.14 155)', color: 'var(--paper)', fontSize: 9, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>✓</div>;
    if (s === 'online') return <Ico name="cloud" size={13} color="var(--ink-3)" />;
    if (s === 'locked') return <Ico name="lock" size={12} color="var(--amber-deep)" />;
    return null;
  };
  return (
    <DesktopWin title="File Explorer" width={820} height={540} platform="win">
      {/* ribbon */}
      <div style={{ background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 14, fontSize: 11.5 }}>
        {['Home', 'Share', 'View'].map((t, i) => (
          <span key={i} style={{ color: i === 0 ? 'var(--ink)' : 'var(--ink-3)', fontWeight: i === 0 ? 600 : 400, padding: '3px 0', borderBottom: i === 0 ? '2px solid var(--amber-deep)' : 'none' }}>{t}</span>
        ))}
      </div>
      <div style={{ background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', padding: '6px 14px', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="chevLeft" size={13} color="var(--ink-3)" />
        <Ico name="chevRight" size={13} color="var(--ink-4)" />
        <div className="bb-input" style={{ flex: 1, padding: '4px 10px', fontSize: 11 }}>
          <span className="t-mono" style={{ color: 'var(--ink-2)' }}>C:\Users\Isa\Beebeeb\investigations</span>
        </div>
        <div className="bb-input" style={{ width: 180, padding: '4px 10px', fontSize: 11 }}>
          <Ico name="search" size={11} color="var(--ink-4)" />
          <input placeholder="Search inside encrypted index…" style={{ fontSize: 11 }} />
        </div>
      </div>
      <div style={{ display: 'flex', height: 'calc(100% - 72px)' }}>
        <div style={{ width: 200, background: 'var(--paper-2)', borderRight: '1px solid var(--line)', padding: '12px 8px', overflow: 'auto' }}>
          <div className="t-label" style={{ fontSize: 9.5, padding: '0 6px 6px' }}>Quick access</div>
          {['Desktop', 'Downloads', 'Documents', 'Pictures'].map((l, i) => (
            <div key={i} style={{ padding: '4px 8px', fontSize: 11.5, color: 'var(--ink-2)', display: 'flex', alignItems: 'center', gap: 6 }}>
              <Ico name="folder" size={10} color="var(--ink-3)" /> {l}
            </div>
          ))}
          <div className="t-label" style={{ fontSize: 9.5, padding: '12px 6px 6px' }}>Beebeeb</div>
          {[
            ['Vault', true, 'folder'],
            ['investigations', true, 'folder', true],
            ['photos', false, 'folder'],
            ['archive', false, 'cloud'],
            ['Shared with me', false, 'users'],
          ].map(([l, bold, ic, active], i) => (
            <div key={i} style={{
              padding: '4px 8px', fontSize: 11.5,
              color: active ? 'var(--ink)' : 'var(--ink-2)',
              fontWeight: active ? 600 : bold ? 500 : 400,
              background: active ? 'var(--paper)' : 'transparent',
              borderLeft: active ? '2px solid var(--amber-deep)' : '2px solid transparent',
              display: 'flex', alignItems: 'center', gap: 6,
              marginLeft: l === 'investigations' ? 12 : 0,
            }}>
              <Ico name={ic} size={10} color={active ? 'var(--amber-deep)' : 'var(--ink-3)'} /> {l}
            </div>
          ))}
        </div>

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
          <div style={{
            display: 'grid', gridTemplateColumns: '20px 1fr 100px 120px 90px',
            gap: 10, padding: '7px 14px',
            background: 'var(--paper-2)', borderBottom: '1px solid var(--line)',
            fontSize: 10.5, color: 'var(--ink-3)', fontWeight: 500,
          }}>
            <div />
            <span>Name</span>
            <span>Status</span>
            <span>Date modified</span>
            <span style={{ textAlign: 'right' }}>Size</span>
          </div>
          <div style={{ flex: 1, overflow: 'auto' }}>
            {files.map((f, i) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '20px 1fr 100px 120px 90px',
                gap: 10, padding: '6px 14px', alignItems: 'center',
                borderBottom: '1px solid var(--line)',
                background: f.state === 'syncing' ? 'var(--amber-bg)' : 'var(--paper)',
              }}>
                {stateIcon(f.state, f.prog)}
                <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                  <Ico name={f.t === 'folder' ? 'folder' : 'file'} size={12} color={f.t === 'folder' ? 'var(--amber-deep)' : 'var(--ink-3)'} />
                  <span style={{ fontSize: 12 }}>{f.n}</span>
                  {f.state === 'locked' && <BBChip style={{ fontSize: 9, padding: '0 6px' }}>one-time</BBChip>}
                </div>
                <span className="t-mono" style={{ fontSize: 10.5, color: f.state === 'syncing' ? 'var(--amber-deep)' : 'var(--ink-3)' }}>
                  {f.state === 'syncing' ? `${f.prog}% · encrypting` : f.state === 'synced' ? 'Available' : f.state === 'online' ? 'Online-only' : 'Locked'}
                </span>
                <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>{f.d}</span>
                <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', textAlign: 'right' }}>{f.sz}</span>
              </div>
            ))}
          </div>
          <div style={{ padding: '6px 14px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10, fontSize: 10.5, color: 'var(--ink-3)' }}>
            <Ico name="shield" size={10} color="var(--amber-deep)" />
            <span>7 items · 1 syncing · End-to-end encrypted · Frankfurt</span>
            <span style={{ marginLeft: 'auto' }} className="t-mono">12.1 GB used</span>
          </div>
        </div>
      </div>
      <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
    </DesktopWin>
  );
}

// ─── Windows 11 tray / taskbar flyout ─────────────────────────────────
function HiWindowsTray() {
  return (
    <div style={{
      width: 400, height: 280, background: 'var(--paper)',
      borderRadius: 10, overflow: 'hidden',
      border: '1px solid var(--line-2)',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.35), 0 0 0 1px var(--line-2)',
      display: 'flex', flexDirection: 'column',
      fontFamily: 'var(--font-sans)',
    }}>
      {/* header */}
      <div style={{
        padding: '12px 14px 10px', display: 'flex', alignItems: 'center', gap: 10,
        borderBottom: '1px solid var(--line)',
      }}>
        <div style={{ width: 26, height: 26, borderRadius: 7, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <BBHex />
        </div>
        <div>
          <div style={{ fontSize: 12.5, fontWeight: 600 }}>Beebeeb</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Up to date · 23.4 GB synced</div>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 2 }}>
          {['–', '✕'].map((c, i) => (
            <div key={i} style={{ width: 24, height: 24, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, color: 'var(--ink-3)' }}>{c}</div>
          ))}
        </div>
      </div>

      {/* activity list */}
      <div style={{ flex: 1, overflow: 'auto', padding: '4px 0' }}>
        {[
          { n: 'story-draft.md', s: 'Encrypting · 72%', ico: 'file', active: true, prog: 72 },
          { n: 'leak-packet.pdf', s: 'Uploaded · 2 min ago', ico: 'shield', ok: true },
          { n: 'interview-03.m4a', s: 'Synced · 4 min ago', ico: 'file', ok: true },
          { n: 'photos/', s: 'Auto-backup paused · not on Wi-Fi', ico: 'pause', warn: true },
        ].map((r, i) => (
          <div key={i} style={{
            display: 'grid', gridTemplateColumns: '28px 1fr auto',
            alignItems: 'center', gap: 10, padding: '8px 14px',
            background: r.active ? 'var(--amber-bg)' : 'transparent',
          }}>
            <div style={{
              width: 22, height: 22, borderRadius: 6, background: 'var(--paper-2)',
              border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Ico name={r.ico} size={11} color={r.active ? 'var(--amber-deep)' : r.warn ? 'var(--ink-3)' : 'var(--ink-3)'} />
            </div>
            <div>
              <div style={{ fontSize: 11.5, fontWeight: 500 }}>{r.n}</div>
              <div className="t-micro" style={{ color: r.warn ? 'oklch(0.55 0.12 55)' : 'var(--ink-3)', marginTop: 2 }}>{r.s}</div>
              {r.active && (
                <div className="bb-progress" style={{ marginTop: 5, height: 2 }}>
                  <div style={{ width: `${r.prog}%` }} />
                </div>
              )}
            </div>
            {r.ok && <span style={{ fontSize: 11, color: 'oklch(0.55 0.14 155)' }}>✓</span>}
          </div>
        ))}
      </div>

      {/* footer bar */}
      <div style={{
        borderTop: '1px solid var(--line)', padding: '8px 14px',
        display: 'flex', alignItems: 'center', gap: 10,
        background: 'var(--paper-2)',
      }}>
        <Ico name="shield" size={11} color="var(--amber-deep)" />
        <span className="t-micro" style={{ color: 'var(--ink-2)' }}>Frankfurt · E2E encrypted</span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <BBBtn size="sm" variant="ghost" style={{ padding: '3px 8px', fontSize: 11 }}>Pause</BBBtn>
          <BBBtn size="sm" variant="ghost" style={{ padding: '3px 8px', fontSize: 11 }}>Settings</BBBtn>
        </div>
      </div>

      {/* caret pointing to tray */}
      <div style={{
        position: 'absolute', bottom: -6, right: 40,
        width: 12, height: 12, background: 'var(--paper-2)',
        borderRight: '1px solid var(--line-2)', borderBottom: '1px solid var(--line-2)',
        transform: 'rotate(45deg)',
      }} />
      <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
    </div>
  );
}

// ─── Windows 11 Settings (Fluent-style) ───────────────────────────────
function HiWindowsSettings() {
  const navSections = [
    ['Beebeeb', [
      ['Account', 'user', false],
      ['Sync', 'cloud', true],
      ['Selective sync', 'folder', false],
      ['Bandwidth', 'bolt', false],
    ]],
    ['Security', [
      ['Vault passphrase', 'lock', false],
      ['Recovery kit', 'key', false],
      ['Linked devices', 'device', false],
    ]],
    ['System', [
      ['Launch', 'play', false],
      ['Explorer integration', 'folder', false],
      ['Advanced', 'cog', false],
    ]],
  ];
  return (
    <DesktopWin title="Beebeeb — Settings" width={1000} height={680} platform="win">
      <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', height: '100%' }}>
        {/* sidebar */}
        <div style={{ background: 'var(--paper-2)', borderRight: '1px solid var(--line)', padding: '16px 10px', overflow: 'auto' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px 14px' }}>
            <BBHex />
            <span style={{ fontSize: 13, fontWeight: 600 }}>beebeeb.io</span>
          </div>
          <div className="bb-input" style={{ padding: '5px 10px', fontSize: 11.5, marginBottom: 12 }}>
            <Ico name="search" size={11} color="var(--ink-4)" />
            <input placeholder="Find a setting…" style={{ fontSize: 11.5 }} />
          </div>
          {navSections.map(([h, items], si) => (
            <div key={si} style={{ marginBottom: 10 }}>
              <div className="t-label" style={{ fontSize: 9.5, padding: '4px 10px 4px' }}>{h}</div>
              {items.map(([l, ico, active], i) => (
                <div key={i} style={{
                  padding: '7px 10px', borderRadius: 6,
                  display: 'flex', alignItems: 'center', gap: 10,
                  fontSize: 12.5, fontWeight: active ? 600 : 400,
                  color: active ? 'var(--ink)' : 'var(--ink-2)',
                  background: active ? 'var(--paper)' : 'transparent',
                  border: active ? '1px solid var(--line)' : '1px solid transparent',
                  borderLeft: active ? '3px solid var(--amber-deep)' : '3px solid transparent',
                }}>
                  <Ico name={ico} size={12} color={active ? 'var(--amber-deep)' : 'var(--ink-3)'} />
                  {l}
                </div>
              ))}
            </div>
          ))}
        </div>

        {/* content */}
        <div style={{ overflow: 'auto', padding: '28px 36px' }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
            <div className="t-h2" style={{ fontSize: 26, fontWeight: 600 }}>Sync</div>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>• All devices up to date</span>
          </div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4, marginBottom: 24 }}>
            Manage what syncs with this PC and how. Files are encrypted on this device before upload.
          </div>

          {/* Status card */}
          <div style={{
            padding: 20, borderRadius: 10,
            background: 'linear-gradient(135deg, var(--amber-bg), var(--paper-2))',
            border: '1px solid var(--amber-deep)', marginBottom: 22,
            display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 20,
          }}>
            {[
              ['State', 'Up to date'],
              ['On this PC', '19.1 GB'],
              ['In vault', '23.4 GB'],
              ['Last sync', '2 min ago'],
            ].map(([k, v]) => (
              <div key={k}>
                <div className="t-label" style={{ fontSize: 9.5 }}>{k}</div>
                <div className="t-mono" style={{ fontSize: 15, fontWeight: 600, marginTop: 4 }}>{v}</div>
              </div>
            ))}
          </div>

          {/* Sync rows */}
          <div style={{ background: 'var(--paper)', borderRadius: 10, border: '1px solid var(--line)' }}>
            {[
              { l: 'Keep files encrypted end-to-end', hint: 'This cannot be disabled. Every byte on disk is ciphertext.', toggle: true, locked: true },
              { l: 'Sync on metered connections', hint: 'When off, Beebeeb waits for Wi-Fi or Ethernet.', toggle: false },
              { l: 'Upload rate limit', hint: 'Auto = adapt to available bandwidth', v: 'Auto', chev: true },
              { l: 'Download rate limit', hint: 'Auto = adapt to available bandwidth', v: 'Auto', chev: true },
              { l: 'Files on Demand (online-only by default)', hint: 'Placeholders appear in File Explorer. Open a file to download.', toggle: true },
              { l: 'Show sync overlays in File Explorer', hint: 'The little ✓, cloud and spinner icons on your files.', toggle: true },
            ].map((r, i, arr) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '1fr 140px',
                padding: '14px 18px', alignItems: 'center', gap: 12,
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
              }}>
                <div>
                  <div style={{ fontSize: 13, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 8 }}>
                    {r.l}
                    {r.locked && <BBChip style={{ fontSize: 9, padding: '0 6px' }}>Locked · by design</BBChip>}
                  </div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 3 }}>{r.hint}</div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  {r.toggle !== undefined && <BBToggle on={r.toggle} />}
                  {r.v && <span className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>{r.v} ▾</span>}
                </div>
              </div>
            ))}
          </div>

          {/* Storage footer */}
          <div style={{ marginTop: 24, padding: 18, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10, display: 'flex', alignItems: 'center', gap: 14 }}>
            <div style={{ width: 32, height: 32, borderRadius: 8, background: 'var(--paper)', border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Ico name="cloud" size={14} color="var(--amber-deep)" />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 500 }}>Free up space</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>Convert files you haven't opened in 30 days to online-only. Keeps them in the vault, removes from disk.</div>
            </div>
            <BBBtn size="sm" variant="primary">Free up 7.3 GB</BBBtn>
          </div>
        </div>
      </div>
    </DesktopWin>
  );
}

Object.assign(window, { HiDesktopPrefs, HiSelectiveSync, HiConflict, HiFirstRun, HiWindowsExplorer, HiWindowsTray, HiWindowsSettings });
