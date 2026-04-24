// hifi-team-photos.jsx — Team A + Photos A hi-fi

function HiTeam() {
  const rows = [
    { name: 'Anna Kovač', email: 'anna@example.eu', role: 'Owner', folders: ['Contracts', 'Financials', 'Design', 'Clients'], color: '#f5b800' },
    { name: 'Marc Dupont', email: 'marc@example.eu', role: 'Admin', folders: ['Contracts', 'Financials', 'Design'], color: '#e85a4f' },
    { name: 'Lena Weber', email: 'lena@example.eu', role: 'Editor', folders: ['Design', 'Clients'], color: '#3b82f6' },
    { name: 'Pieter Jansen', email: 'pieter@example.eu', role: 'Viewer', folders: ['Design'], color: '#a855f7' },
    { name: 'ACME (client)', email: 'portal link · expires Apr 30', role: 'Client', folders: ['Clients / ACME'], color: '#0f766e' },
  ];
  const roleChip = (r) => {
    if (r === 'Owner') return <BBChip variant="amber">{r}</BBChip>;
    if (r === 'Admin') return <BBChip variant="filled">{r}</BBChip>;
    if (r === 'Client') return <BBChip variant="green">{r}</BBChip>;
    return <BBChip>{r}</BBChip>;
  };
  return (
    <div className="bb-card elevated" style={{ width: 880 }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="users" size={15} />
        <div>
          <div className="t-h3">Acme Studio</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>12 members · 3 client portals</div>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <div className="bb-input" style={{ width: 220, padding: '6px 10px' }}>
            <Ico name="search" size={12} color="var(--ink-4)" />
            <input placeholder="Search people…" />
          </div>
          <BBBtn variant="amber" icon={<Ico name="plus" size={12} />}>Invite</BBBtn>
        </div>
      </div>

      <div style={{
        padding: '8px 22px', borderBottom: '1px solid var(--line)', background: 'var(--paper-2)',
        display: 'grid', gridTemplateColumns: '1.4fr 0.8fr 1.8fr 40px', gap: 14,
      }}>
        <span className="t-label">Member</span>
        <span className="t-label">Role</span>
        <span className="t-label">Has access to</span>
        <span />
      </div>

      <div>
        {rows.map((r, i) => (
          <div key={i} className="bb-row" style={{ gridTemplateColumns: '1.4fr 0.8fr 1.8fr 40px', gap: 14, padding: '12px 22px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <div style={{
                width: 32, height: 32, borderRadius: 999, background: r.color,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                color: 'white', fontWeight: 600, fontSize: 12,
                boxShadow: '0 0 0 2px var(--paper)',
              }}>
                {r.name.split(' ').map(w => w[0]).join('').slice(0, 2)}
              </div>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>{r.name}</div>
                <div className="t-micro t-mono" style={{ color: 'var(--ink-3)' }}>{r.email}</div>
              </div>
            </div>
            <div>{roleChip(r.role)}</div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 5 }}>
              {r.folders.map(f => (
                <span key={f} style={{
                  display: 'inline-flex', alignItems: 'center', gap: 4,
                  padding: '2px 7px 2px 5px', borderRadius: 999,
                  background: 'var(--paper-2)', border: '1px solid var(--line)',
                  fontSize: 11.5, color: 'var(--ink-2)',
                }}>
                  <Ico name="folder" size={10} color="var(--amber-deep)" />
                  {f}
                </span>
              ))}
              <span style={{
                display: 'inline-flex', alignItems: 'center', gap: 3,
                padding: '2px 7px', borderRadius: 999,
                fontSize: 11, color: 'var(--ink-4)', cursor: 'pointer',
              }}>
                <Ico name="plus" size={10} /> add
              </span>
            </div>
            <BBBtn size="sm" variant="ghost" icon={<Ico name="more" size={14} />} />
          </div>
        ))}
      </div>
    </div>
  );
}

function HiPhotos() {
  const groups = [
    { date: 'Monday · Apr 14', place: 'Lisbon', count: 10, feat: [3] },
    { date: 'Sunday · Apr 6', place: 'Paris', count: 8, feat: [] },
    { date: 'Apr 1 — Mar 28', place: null, count: 12, feat: [5] },
  ];
  const tile = (i, featured) => (
    <div key={i} className="bb-ph" style={{
      aspectRatio: '1',
      background: featured ? 'linear-gradient(135deg, oklch(0.88 0.12 80), oklch(0.72 0.18 55))' : undefined,
      backgroundImage: featured ? undefined : 'repeating-linear-gradient(135deg, oklch(0.84 0.01 80) 0 1px, transparent 1px 8px)',
      borderRadius: 4,
      position: 'relative', overflow: 'hidden',
    }}>
      {featured && <div style={{ position: 'absolute', right: 4, top: 4, width: 16, height: 16, borderRadius: 999, background: 'rgba(0,0,0,0.35)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <Ico name="star" size={9} color="white" />
      </div>}
    </div>
  );
  return (
    <div className="bb-card elevated" style={{ width: 900, height: 620, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 12 }}>
        <Ico name="image" size={15} />
        <div>
          <div className="t-h3">Photos</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}><span className="t-mono num-tabular">2,408</span> items · <span className="t-mono num-tabular">32.4 GB</span></div>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 4, padding: 3, background: 'var(--paper-2)', borderRadius: 'var(--r-2)', border: '1px solid var(--line)' }}>
          {['All', 'Albums', 'People', 'Places'].map((t, i) => (
            <span key={t} style={{
              padding: '4px 10px', borderRadius: 4, fontSize: 12,
              background: i === 0 ? 'var(--paper)' : 'transparent',
              boxShadow: i === 0 ? 'var(--shadow-1)' : 'none',
              fontWeight: i === 0 ? 600 : 400, color: i === 0 ? 'var(--ink)' : 'var(--ink-3)',
            }}>{t}</span>
          ))}
        </div>
        <BBBtn size="sm" icon={<Ico name="upload" size={12} />}>Upload</BBBtn>
      </div>

      <div className="bb-scroll" style={{ flex: 1, padding: '18px 22px' }}>
        {groups.map((g, gi) => (
          <div key={gi} style={{ marginBottom: 24 }}>
            <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 10, gap: 10 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>{g.date}</div>
              {g.place && <div className="t-micro" style={{ color: 'var(--ink-3)' }}>· {g.place}</div>}
              <div className="t-mono t-micro" style={{ color: 'var(--ink-4)', marginLeft: 'auto' }}>{g.count} photos</div>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 4 }}>
              {Array.from({ length: g.count }).map((_, i) => tile(i, g.feat.includes(i)))}
            </div>
          </div>
        ))}
      </div>

      <div style={{
        padding: '8px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)',
        display: 'flex', alignItems: 'center', gap: 14, fontSize: 11, color: 'var(--ink-3)',
      }}>
        <Ico name="shield" size={12} color="var(--amber-deep)" /> All photos E2E encrypted · EXIF stripped on upload
        <BBRegionBadge region="Amsterdam" />
      </div>
    </div>
  );
}

Object.assign(window, { HiTeam, HiPhotos });
