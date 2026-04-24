// flows-team-photos.jsx — Team sharing/permissions + Photos gallery

// ─── Team A — Table with role + folders-access columns ───────────────
function TeamA() {
  const rows = [
    ['Anna Kovač', 'anna@example.eu', 'Owner', ['Contracts', 'Financials', 'Design', 'Clients']],
    ['Marc Dupont', 'marc@example.eu', 'Admin', ['Contracts', 'Financials', 'Design']],
    ['Lena Weber', 'lena@example.eu', 'Editor', ['Design', 'Clients']],
    ['Pieter Jansen', 'pieter@example.eu', 'Viewer', ['Design']],
    ['Client · ACME', 'portal link', 'Client', ['Clients/ACME']],
  ];
  return (
    <div className="wf-card" style={{ width: 680, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 18px', borderBottom: '1.2px solid var(--ink)' }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Team · Acme Studio</span>
        <Chip amber style={{ marginLeft: 10 }}>⬡ 12 members</Chip>
        <Btn amber small style={{ marginLeft: 'auto' }}>＋ Invite</Btn>
      </div>
      <div style={{ padding: '8px 18px', display: 'grid', gridTemplateColumns: '1.3fr 0.7fr 1.6fr', gap: 10, fontFamily: 'var(--mono)', fontSize: 9, textTransform: 'uppercase', color: 'var(--ink-3)', letterSpacing: 0.6, borderBottom: '1px dashed rgba(26,26,26,0.25)' }}>
        <span>Member</span><span>Role</span><span>Has access to</span>
      </div>
      {rows.map(([n, e, role, folders], i) => (
        <div key={i} style={{ padding: '10px 18px', display: 'grid', gridTemplateColumns: '1.3fr 0.7fr 1.6fr', gap: 10, alignItems: 'center', borderBottom: '1px dashed rgba(26,26,26,0.18)', fontSize: 12 }}>
          <div>
            <div style={{ fontWeight: 600 }}>{n}</div>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{e}</div>
          </div>
          <Chip amber={role === 'Owner'} filled={role === 'Admin'}>{role}</Chip>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
            {folders.map(f => (
              <span key={f} style={{
                fontFamily: 'var(--mono)', fontSize: 10, padding: '2px 6px',
                border: '1px solid var(--ink)', background: 'var(--paper-2)',
              }}>▢ {f}</span>
            ))}
            <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', padding: '2px 4px' }}>+ edit</span>
          </div>
        </div>
      ))}
    </div>
  );
}

// ─── Team B — Permission matrix (folders × roles) ──────────────────────
function TeamB() {
  const folders = ['Contracts', 'Financials', 'Design', 'Clients'];
  const roles = ['Owner', 'Admin', 'Editor', 'Viewer', 'Client'];
  const M = [
    ['R/W','R/W','R/W','R/W','—'],
    ['R/W','R/W','R','R','—'],
    ['R','R','R/W','R/W','—'],
    ['R','—','R','R','R-link'],
  ];
  return (
    <div className="wf-card" style={{ width: 640, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 18px', borderBottom: '1.2px solid var(--ink)' }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Permissions matrix</span>
        <Chip style={{ marginLeft: 'auto' }}>folders × roles</Chip>
      </div>
      <div style={{ padding: 18 }}>
        <div style={{ display: 'grid', gridTemplateColumns: `140px repeat(${roles.length}, 1fr)`, fontFamily: 'var(--mono)', fontSize: 10 }}>
          <div></div>
          {roles.map(r => <div key={r} style={{ padding: '0 6px 8px', textAlign: 'center', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.5 }}>{r}</div>)}
          {folders.map((f, r) => (
            <React.Fragment key={f}>
              <div style={{ padding: '10px 4px', fontWeight: 600, borderTop: '1px dashed rgba(26,26,26,0.2)', fontFamily: 'var(--sans)', fontSize: 12 }}>{f}</div>
              {M[r].map((v, c) => (
                <div key={c} style={{
                  padding: '10px 0', textAlign: 'center',
                  borderTop: '1px dashed rgba(26,26,26,0.2)',
                  background: v === 'R/W' ? 'var(--amber-soft)' : v === '—' ? 'transparent' : 'var(--paper-2)',
                  color: v === '—' ? 'var(--ink-4)' : 'var(--ink)',
                  fontSize: 10,
                }}>{v}</div>
              ))}
            </React.Fragment>
          ))}
        </div>
        <div className="wf-annot" style={{ marginTop: 12, fontSize: 13 }}>
          ↑ amber = full access. hover to edit a cell.
        </div>
      </div>
    </div>
  );
}

// ─── Team C — Cards with avatar rows (people-first) ────────────────────
function TeamC() {
  return (
    <div className="wf-card" style={{ width: 640, fontFamily: 'var(--sans)', overflow: 'hidden', padding: 20 }}>
      <div className="wf-row" style={{ marginBottom: 14 }}>
        <div style={{ fontFamily: 'var(--hand)', fontSize: 22 }}>Your team</div>
        <Chip amber style={{ marginLeft: 'auto' }}>⬡ 12 members · 3 clients</Chip>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
        {[
          { title: 'Owners & Admins', count: 3, region: 'FRA' },
          { title: 'Editors', count: 5, region: 'FRA · AMS' },
          { title: 'Viewers', count: 4, region: 'AMS' },
          { title: 'Client portals', count: 3, region: 'mixed' },
        ].map((g, i) => (
          <div key={i} className="wf-card" style={{ padding: 14, boxShadow: '2px 2px 0 rgba(26,26,26,0.85)' }}>
            <div className="wf-row" style={{ marginBottom: 10 }}>
              <span style={{ fontSize: 12, fontWeight: 600 }}>{g.title}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', marginLeft: 'auto' }}>{g.count}</span>
            </div>
            <div className="wf-row" style={{ gap: -4, marginBottom: 10 }}>
              {Array.from({ length: Math.min(g.count, 5) }).map((_, k) => (
                <div key={k} style={{
                  width: 26, height: 26,
                  clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)',
                  background: k === 0 ? 'var(--amber)' : 'var(--paper-2)',
                  border: '1px solid var(--ink)',
                  marginLeft: k === 0 ? 0 : -6,
                }} />
              ))}
            </div>
            <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{g.region}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Photos A — Classic grid ───────────────────────────────────────────
function PhotosA() {
  return (
    <div className="wf-card" style={{ width: 640, height: 420, fontFamily: 'var(--sans)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
      <div className="wf-row" style={{ padding: '12px 16px', borderBottom: '1.2px solid var(--ink)' }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Photos · 2,408</span>
        <div className="wf-row" style={{ gap: 6, marginLeft: 'auto' }}>
          <Chip filled>All</Chip><Chip>Albums</Chip><Chip>Places</Chip>
        </div>
      </div>
      <div className="wf-scroll" style={{ padding: 12, flex: 1 }}>
        <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', margin: '4px 4px 8px', textTransform: 'uppercase', letterSpacing: 0.6 }}>Apr 14 · Lisbon</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 4, marginBottom: 14 }}>
          {Array.from({ length: 10 }).map((_, i) => (
            <div key={i} className="wf-ph" style={{ aspectRatio: '1', background: i === 3 ? 'var(--amber-soft)' : undefined }} />
          ))}
        </div>
        <div style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)', margin: '4px 4px 8px', textTransform: 'uppercase', letterSpacing: 0.6 }}>Apr 6 · Paris</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 4 }}>
          {Array.from({ length: 8 }).map((_, i) => (
            <div key={i} className="wf-ph" style={{ aspectRatio: '1' }} />
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Photos B — Timeline with map strip ────────────────────────────────
function PhotosB() {
  return (
    <div className="wf-card" style={{ width: 640, height: 420, fontFamily: 'var(--sans)', overflow: 'hidden', display: 'flex' }}>
      <div style={{ width: 140, borderRight: '1.2px solid var(--ink)', background: 'var(--paper-2)', padding: 14 }}>
        <div style={{ fontFamily: 'var(--hand)', fontSize: 18, marginBottom: 10 }}>Timeline</div>
        {['2026', '2025', '2024', '2023'].map((y, i) => (
          <div key={y} style={{ padding: '6px 0', fontSize: 12, fontWeight: i === 0 ? 600 : 400, borderBottom: '1px dashed rgba(26,26,26,0.2)' }}>{y}</div>
        ))}
        <Rule dashed style={{ margin: '14px 0' }} />
        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.5, marginBottom: 6 }}>Places</div>
        {['Lisbon', 'Paris', 'Berlin', 'Athens'].map(p => (
          <div key={p} style={{ fontSize: 11, padding: '3px 0' }}>⬡ {p}</div>
        ))}
      </div>
      <div style={{ flex: 1, padding: 14, display: 'flex', flexDirection: 'column', gap: 10, overflow: 'hidden' }}>
        <Placeholder h={90} label="map strip · 4 cities" />
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 4 }}>
          {Array.from({ length: 12 }).map((_, i) => (
            <div key={i} className="wf-ph" style={{ aspectRatio: '1', background: i === 2 ? 'var(--amber-soft)' : undefined }} />
          ))}
        </div>
        <div className="wf-annot" style={{ fontSize: 13 }}>all photos encrypted · EXIF stripped before upload (opt-in)</div>
      </div>
    </div>
  );
}

// ─── Photos C — Hex album tiles ────────────────────────────────────────
function PhotosC() {
  return (
    <div className="wf-card" style={{ width: 640, height: 420, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 18px', borderBottom: '1.2px solid var(--ink)' }}>
        <span style={{ fontFamily: 'var(--hand)', fontSize: 20 }}>Albums</span>
        <Btn amber small style={{ marginLeft: 'auto' }}>＋ New album</Btn>
      </div>
      <div style={{ padding: 18, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 14 }}>
        {['Lisbon 2026', 'Studio · archive', 'Family', 'Wedding', 'Team offsite', 'Board trip', 'Screenshots', 'Inbox'].map((title, i) => (
          <div key={i} style={{ textAlign: 'center' }}>
            <div style={{
              aspectRatio: '1 / 1.15',
              clipPath: 'polygon(50% 0, 100% 25%, 100% 75%, 50% 100%, 0 75%, 0 25%)',
              background: i % 3 === 0 ? 'var(--amber-soft)' : 'var(--paper-2)',
              border: '1.1px solid var(--ink)',
              backgroundImage: 'repeating-linear-gradient(135deg, rgba(26,26,26,0.08) 0 1px, transparent 1px 7px)',
              marginBottom: 6,
            }} />
            <div style={{ fontSize: 11, fontWeight: 600 }}>{title}</div>
            <div style={{ fontSize: 10, fontFamily: 'var(--mono)', color: 'var(--ink-3)' }}>{20 + i * 17} photos</div>
          </div>
        ))}
      </div>
    </div>
  );
}

Object.assign(window, { TeamA, TeamB, TeamC, PhotosA, PhotosB, PhotosC });
