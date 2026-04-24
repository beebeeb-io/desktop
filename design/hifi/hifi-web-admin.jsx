// hifi-web-admin.jsx — Notifications inbox + Team admin dashboard + Migration wizard

// ─── Notifications inbox dropdown ──────────────────────────────────────────
function HiNotifications() {
  const groups = [
    { h: 'Today', items: [
      { ico: 'share', who: 'Pieter J.', what: 'shared ', target: 'Q2 Financials', sub: 'with you', t: '12m', unread: true },
      { ico: 'users', who: 'Beebeeb', what: 'added a new device · ', target: 'Pixel 9', sub: 'Amsterdam · verify if this was you', t: '1h', unread: true, danger: true },
      { ico: 'clock', who: 'Marc D.', what: 'edited ', target: 'term-sheet-v3.docx', sub: '3 new versions', t: '4h', unread: true },
    ]},
    { h: 'Yesterday', items: [
      { ico: 'key', who: 'Beebeeb', what: 'Recovery phrase reveal — ', target: 'MacBook Pro', sub: 'Frankfurt · you', t: '1d' },
      { ico: 'lock', who: 'Lena W.', what: 'joined the team ', target: 'Acme Studio', sub: 'via invite link', t: '1d' },
    ]},
    { h: 'Earlier', items: [
      { ico: 'cloud', who: 'Beebeeb', what: 'Monthly usage report ready · ', target: '23.4 GB / 2 TB', sub: 'no anomalies detected', t: '4d' },
      { ico: 'shield', who: 'Beebeeb', what: 'Warrant canary signed ', target: 'for April 2026', sub: 'no data requests received', t: '1w' },
    ]},
  ];
  return (
    <div style={{
      width: 760, height: 600, background: 'var(--paper-2)',
      borderRadius: 12, position: 'relative', overflow: 'hidden',
      border: '1px solid var(--line-2)',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.22)',
    }}>
      {/* faint app chrome behind */}
      <div style={{ height: 52, background: 'var(--paper)', borderBottom: '1px solid var(--line)', padding: '0 20px', display: 'flex', alignItems: 'center', gap: 14 }}>
        <BBLogo size={13} />
        <div className="bb-input" style={{ padding: '5px 10px', fontSize: 11, maxWidth: 360, flex: 1, opacity: 0.7 }}>
          <Ico name="search" size={11} color="var(--ink-4)" />
          <input placeholder="Search encrypted filenames…" />
          <BBKbd>⌘K</BBKbd>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ position: 'relative' }}>
            <Ico name="cloud" size={14} color="var(--ink-2)" />
            <span style={{ position: 'absolute', top: -4, right: -4, width: 7, height: 7, borderRadius: '50%', background: 'var(--amber-deep)' }} />
          </div>
          <div style={{ width: 26, height: 26, borderRadius: '50%', background: 'oklch(0.7 0.1 55)', color: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, fontWeight: 600 }}>IM</div>
        </div>
      </div>

      {/* Dropdown panel */}
      <div style={{
        position: 'absolute', top: 60, right: 50, width: 420,
        background: 'var(--paper)', borderRadius: 12,
        border: '1px solid var(--line-2)',
        boxShadow: '0 22px 60px -12px rgba(0,0,0,0.22)',
        overflow: 'hidden', maxHeight: 520,
      }}>
        <div style={{
          padding: '14px 16px', display: 'flex', alignItems: 'center', gap: 10,
          borderBottom: '1px solid var(--line)',
        }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Activity</div>
          <BBChip variant="amber">3 new</BBChip>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
            <BBChip>All</BBChip>
            <BBChip style={{ opacity: 0.6 }}>Mentions</BBChip>
            <BBChip style={{ opacity: 0.6 }}>Security</BBChip>
          </div>
        </div>
        <div style={{ maxHeight: 420, overflow: 'auto' }}>
          {groups.map((g, gi) => (
            <div key={gi}>
              <div className="t-label" style={{ padding: '10px 16px 4px', fontSize: 9.5, background: 'var(--paper-2)' }}>{g.h}</div>
              {g.items.map((it, i) => (
                <div key={i} style={{
                  display: 'grid', gridTemplateColumns: '30px 1fr auto',
                  gap: 10, padding: '10px 16px',
                  background: it.unread ? 'var(--amber-bg)' : 'transparent',
                  borderBottom: '1px solid var(--line)',
                  alignItems: 'flex-start',
                }}>
                  <div style={{
                    width: 26, height: 26, borderRadius: 7,
                    background: it.danger ? 'oklch(0.96 0.04 28)' : 'var(--paper-2)',
                    border: `1px solid ${it.danger ? 'oklch(0.85 0.1 28)' : 'var(--line)'}`,
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                  }}>
                    <Ico name={it.ico} size={11} color={it.danger ? 'oklch(0.5 0.16 28)' : 'var(--ink-2)'} />
                  </div>
                  <div>
                    <div style={{ fontSize: 12 }}>
                      <span style={{ fontWeight: 600 }}>{it.who}</span>
                      <span style={{ color: 'var(--ink-2)' }}> {it.what}</span>
                      <span style={{ fontWeight: 500 }}>{it.target}</span>
                    </div>
                    <div className="t-micro" style={{ color: it.danger ? 'oklch(0.55 0.12 28)' : 'var(--ink-3)', marginTop: 2, lineHeight: 1.4 }}>{it.sub}</div>
                  </div>
                  <span className="t-micro" style={{ color: 'var(--ink-4)', whiteSpace: 'nowrap' }}>{it.t}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
        <div style={{ padding: '10px 16px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', display: 'flex', gap: 10, alignItems: 'center' }}>
          <BBBtn size="sm" variant="ghost"><Ico name="check" size={11} /> Mark all read</BBBtn>
          <span className="t-micro" style={{ color: 'var(--ink-3)', marginLeft: 'auto' }}>Archive after 30 days</span>
        </div>
      </div>
    </div>
  );
}

// ─── Team admin dashboard ──────────────────────────────────────────────────
function HiTeamAdmin() {
  const stats = [
    { l: 'Members', v: '8 / 10', sub: '2 pending invites' },
    { l: 'Storage', v: '312 GB', sub: 'of 2 TB · 15%' },
    { l: 'Files', v: '2,834', sub: '+147 this week' },
    { l: 'Monthly cost', v: '€48.00', sub: 'next charge 1 May' },
  ];
  const members = [
    { n: 'Isa Marchetti', e: 'isa@example.eu', r: 'Owner', last: 'active', i: 'IM', c: 'oklch(0.7 0.1 55)' },
    { n: 'Marc Dubois', e: 'marc@example.eu', r: 'Admin', last: '2m ago', i: 'MD', c: 'oklch(0.7 0.1 220)' },
    { n: 'Lena Wagner', e: 'lena@example.eu', r: 'Member', last: '1h ago', i: 'LW', c: 'oklch(0.7 0.1 155)' },
    { n: 'Pieter Jansen', e: 'pieter@example.eu', r: 'Member', last: 'yesterday', i: 'PJ', c: 'oklch(0.7 0.1 280)' },
    { n: 'Anna Koch', e: 'anna@example.eu', r: 'Member', last: '3d ago', i: 'AK', c: 'oklch(0.7 0.1 25)' },
    { n: 'editor@example.eu', e: '—', r: 'Invited', last: 'Sent 2d ago', i: '?', c: 'var(--ink-4)', pending: true },
  ];
  const events = [
    { ico: 'upload', who: 'Marc D.', what: 'uploaded 3 files to Contracts/', t: '2m ago' },
    { ico: 'share', who: 'Lena W.', what: 'shared board-deck-apr.pdf externally', t: '1h ago', flag: true },
    { ico: 'key', who: 'Pieter J.', what: 'rotated recovery phrase', t: 'yesterday' },
    { ico: 'users', who: 'Isa M.', what: 'invited editor@example.eu', t: '2d ago' },
    { ico: 'lock', who: 'Anna K.', what: 'enabled passkey', t: '3d ago' },
  ];
  return (
    <div style={{
      width: 1180, background: 'var(--paper)', borderRadius: 12, overflow: 'hidden',
      border: '1px solid var(--line-2)', boxShadow: '0 22px 60px -18px rgba(0,0,0,0.18)',
      display: 'grid', gridTemplateColumns: '220px 1fr',
    }}>
      <div style={{ background: 'var(--paper-2)', borderRight: '1px solid var(--line)', padding: 16 }}>
        <BBLogo size={13} />
        <div style={{ marginTop: 20 }}>
          {[
            ['Drive', 'folder', false],
            ['Team', 'users', true],
            ['Activity', 'clock', false],
            ['Security', 'shield', false],
            ['Billing', 'cloud', false],
            ['Compliance', 'check', false],
          ].map(([l, ico, active]) => (
            <div key={l} className={`bb-side-item ${active ? 'active' : ''}`}>
              <span className="bb-side-icon"><Ico name={ico} size={12} /></span>
              {l}
            </div>
          ))}
        </div>
      </div>

      <div style={{ padding: 24, overflow: 'hidden' }}>
        {/* header */}
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 14 }}>
          <div className="t-h2" style={{ fontSize: 22, fontWeight: 600, letterSpacing: -0.3 }}>Acme Studio</div>
          <BBChip variant="filled">Team plan</BBChip>
          <BBRegionBadge region="Frankfurt, DE" />
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <BBBtn size="sm" variant="ghost">Settings</BBBtn>
            <BBBtn size="sm" variant="amber"><Ico name="plus" size={11} color="var(--ink)" /> Invite</BBBtn>
          </div>
        </div>

        {/* stats */}
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 14, marginTop: 18 }}>
          {stats.map((s, i) => (
            <div key={i} className="bb-card" style={{ padding: 16 }}>
              <div className="t-label">{s.l}</div>
              <div className="t-mono" style={{ fontSize: 22, fontWeight: 600, marginTop: 4, letterSpacing: -0.3 }}>{s.v}</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4 }}>{s.sub}</div>
            </div>
          ))}
        </div>

        {/* grid */}
        <div style={{ display: 'grid', gridTemplateColumns: '1.35fr 1fr', gap: 16, marginTop: 18 }}>
          {/* members */}
          <div className="bb-card" style={{ padding: 0, overflow: 'hidden' }}>
            <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center' }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>Members</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginLeft: 8 }}>8 active · 1 pending</div>
              <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
                <BBChip>All</BBChip>
                <BBChip style={{ opacity: 0.55 }}>Admins</BBChip>
                <BBChip style={{ opacity: 0.55 }}>Pending</BBChip>
              </div>
            </div>
            {members.map((m, i) => (
              <div key={i} style={{
                display: 'grid', gridTemplateColumns: '28px 1fr 90px 80px 20px',
                alignItems: 'center', gap: 12, padding: '10px 16px',
                borderBottom: i < members.length - 1 ? '1px solid var(--line)' : 'none',
                opacity: m.pending ? 0.7 : 1,
              }}>
                <div style={{
                  width: 26, height: 26, borderRadius: '50%',
                  background: m.c, color: 'var(--paper)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 10, fontWeight: 600,
                  border: m.pending ? '1px dashed var(--line-2)' : 'none',
                  ...(m.pending && { background: 'transparent', color: 'var(--ink-3)' }),
                }}>{m.i}</div>
                <div>
                  <div style={{ fontSize: 12, fontWeight: 500 }}>{m.n}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{m.e}</div>
                </div>
                <BBChip variant={m.r === 'Owner' ? 'amber' : m.r === 'Admin' ? 'filled' : null} style={{ fontSize: 10 }}>{m.r}</BBChip>
                <span className="t-micro" style={{ color: 'var(--ink-3)' }}>{m.last}</span>
                <Ico name="more" size={12} color="var(--ink-3)" />
              </div>
            ))}
          </div>

          {/* activity */}
          <div className="bb-card" style={{ padding: 0, overflow: 'hidden' }}>
            <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center' }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>Team activity</div>
              <span className="t-micro" style={{ color: 'var(--ink-3)', marginLeft: 'auto' }}>Audit-grade · exportable</span>
            </div>
            <div style={{ padding: '4px 0' }}>
              {events.map((e, i) => (
                <div key={i} style={{
                  display: 'grid', gridTemplateColumns: '22px 1fr auto', gap: 10,
                  padding: '10px 16px', alignItems: 'flex-start',
                  background: e.flag ? 'oklch(0.97 0.04 50 / 0.6)' : 'transparent',
                }}>
                  <div style={{
                    width: 22, height: 22, borderRadius: 5, flexShrink: 0,
                    background: 'var(--paper-2)', border: '1px solid var(--line)',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                  }}>
                    <Ico name={e.ico} size={10} color={e.flag ? 'oklch(0.55 0.14 55)' : 'var(--ink-3)'} />
                  </div>
                  <div style={{ fontSize: 11.5 }}>
                    <span style={{ fontWeight: 500 }}>{e.who}</span>
                    <span style={{ color: 'var(--ink-2)' }}> {e.what}</span>
                    {e.flag && <BBChip variant="amber" style={{ fontSize: 9, marginLeft: 6, padding: '0 5px' }}>review</BBChip>}
                  </div>
                  <span className="t-micro" style={{ color: 'var(--ink-4)', whiteSpace: 'nowrap' }}>{e.t}</span>
                </div>
              ))}
              <div style={{ padding: '10px 16px', borderTop: '1px solid var(--line)' }}>
                <BBBtn size="sm" variant="ghost" style={{ width: '100%', justifyContent: 'center' }}>
                  View full audit log <Ico name="chevRight" size={10} />
                </BBBtn>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Migration wizard ──────────────────────────────────────────────────────
function HiMigrationWizard() {
  return (
    <div style={{
      width: 760, background: 'var(--paper)', borderRadius: 14,
      border: '1px solid var(--line-2)', overflow: 'hidden',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.18)',
    }}>
      {/* step rail */}
      <div style={{
        padding: '14px 24px', borderBottom: '1px solid var(--line)',
        display: 'flex', alignItems: 'center', gap: 16,
        background: 'var(--paper-2)',
      }}>
        {[
          ['Source', true, true],
          ['Select', true, false],
          ['Encrypt + upload', false, false, 'now'],
          ['Verify', false, false],
          ['Done', false, false],
        ].map(([l, done, checked, live], i, arr) => (
          <React.Fragment key={i}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div style={{
                width: 20, height: 20, borderRadius: '50%',
                background: done ? 'var(--ink)' : live ? 'var(--amber)' : 'transparent',
                border: `1px solid ${done || live ? 'var(--ink)' : 'var(--line-2)'}`,
                color: done ? 'var(--paper)' : 'var(--ink)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 10, fontWeight: 600,
              }}>
                {done ? '✓' : i + 1}
              </div>
              <span style={{ fontSize: 11.5, fontWeight: live ? 600 : 400, color: live || done ? 'var(--ink)' : 'var(--ink-3)' }}>{l}</span>
            </div>
            {i < arr.length - 1 && <div style={{ flex: 1, height: 1, background: done ? 'var(--ink)' : 'var(--line)' }} />}
          </React.Fragment>
        ))}
      </div>

      <div style={{ padding: 28 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 9,
            background: 'linear-gradient(135deg, #4285F4, #34A853)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'white', fontSize: 16, fontWeight: 700,
          }}>G</div>
          <div>
            <div style={{ fontSize: 17, fontWeight: 600, letterSpacing: -0.2 }}>Moving you in from Google Drive</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>isa.marchetti@gmail.com · 342 files · 18.4 GB</div>
          </div>
          <BBChip variant="green" style={{ marginLeft: 'auto' }}><span className="dot" /> Connected</BBChip>
        </div>

        {/* Live progress */}
        <div style={{
          marginTop: 20, padding: 20,
          background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)',
          borderRadius: 12,
        }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
            <div className="t-h3" style={{ fontSize: 18, fontWeight: 600 }}>Encrypting & uploading</div>
            <span className="t-mono" style={{ fontSize: 12, color: 'var(--ink-2)', marginLeft: 'auto' }}>142 / 342 files · 6.7 GB / 18.4 GB</span>
          </div>
          <div className="bb-progress" style={{ marginTop: 12, height: 6 }}>
            <div style={{ width: '41%' }} />
          </div>
          <div style={{ display: 'flex', gap: 20, marginTop: 14, alignItems: 'center' }}>
            <div>
              <div className="t-label" style={{ fontSize: 9.5 }}>Currently</div>
              <div className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-2)', marginTop: 2 }}>port-contract-draft-03.pdf</div>
            </div>
            <div style={{ marginLeft: 'auto' }}>
              <div className="t-label" style={{ fontSize: 9.5, textAlign: 'right' }}>Est. remaining</div>
              <div className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-2)', marginTop: 2 }}>~ 8 min</div>
            </div>
          </div>
        </div>

        {/* File-type breakdown */}
        <div style={{ marginTop: 22 }}>
          <div className="t-label" style={{ marginBottom: 10 }}>By file type</div>
          {[
            ['Documents', 184, 184, 'var(--ink)'],
            ['Photos', 96, 54, '#3b82f6'],
            ['Videos', 22, 0, '#a855f7'],
            ['Other', 40, 18, 'var(--ink-3)'],
          ].map(([l, tot, done, c], i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '120px 1fr 120px', alignItems: 'center', gap: 14, marginBottom: 8 }}>
              <div style={{ fontSize: 12 }}>{l}</div>
              <div style={{ height: 6, background: 'var(--paper-2)', borderRadius: 3, overflow: 'hidden' }}>
                <div style={{ width: `${(done / tot) * 100}%`, height: '100%', background: c, transition: 'width 300ms' }} />
              </div>
              <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', textAlign: 'right' }}>{done} / {tot}</div>
            </div>
          ))}
        </div>

        {/* How it works */}
        <div style={{
          marginTop: 20, padding: 14,
          background: 'var(--paper-2)', border: '1px solid var(--line)',
          borderRadius: 10, display: 'flex', gap: 12, alignItems: 'flex-start',
        }}>
          <Ico name="shield" size={14} color="var(--amber-deep)" style={{ marginTop: 2 }} />
          <div>
            <div style={{ fontSize: 12, fontWeight: 600 }}>Your files never touch our servers unencrypted</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4, lineHeight: 1.5 }}>
              Transfer runs from your machine. Beebeeb streams directly to Google Drive, encrypts each file
              locally, then uploads ciphertext to Frankfurt. Originals stay in Google until you say delete.
            </div>
          </div>
        </div>

        <div style={{ marginTop: 22, display: 'flex', gap: 10 }}>
          <BBBtn variant="ghost">Pause</BBBtn>
          <BBBtn variant="ghost">Run in background</BBBtn>
          <span style={{ marginLeft: 'auto' }}>
            <BBBtn variant="primary" size="md" style={{ opacity: 0.5 }}>Continue</BBBtn>
          </span>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiNotifications, HiTeamAdmin, HiMigrationWizard });
