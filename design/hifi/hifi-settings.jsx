// hifi-settings.jsx — Wave 4: settings, shared with me, activity

// ─── Settings shell ─────────────────────────────────────
function SettingsShell({ activeSection = 'profile', children }) {
  const nav = [
    ['profile', 'Profile', 'users'],
    ['account', 'Account & security', 'shield'],
    ['devices', 'Devices', 'cloud'],
    ['notifications', 'Notifications', 'clock'],
    ['language', 'Language & region', 'settings'],
    ['appearance', 'Appearance', 'star'],
    ['storage', 'Storage & data', 'folder'],
    ['billing', 'Plan & billing', 'file'],
    ['advanced', 'Advanced', 'key'],
  ];
  return (
    <div style={{
      width: 1040, background: 'var(--paper)', borderRadius: 'var(--r-3)',
      border: '1px solid var(--line-2)', boxShadow: 'var(--shadow-2)',
      display: 'flex', overflow: 'hidden', minHeight: 620,
    }}>
      <div style={{ width: 220, background: 'var(--paper-2)', borderRight: '1px solid var(--line)' }}>
        <div style={{ padding: '16px 16px 10px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Ico name="settings" size={13} color="var(--ink-3)" />
          <div className="t-h3" style={{ fontSize: 14 }}>Settings</div>
        </div>
        <div style={{ padding: '6px 12px' }}>
          {nav.map(([id, label, ic]) => (
            <div key={id} className={'bb-side-item' + (id === activeSection ? ' active' : '')}>
              <span className="bb-side-icon"><Ico name={ic} size={12} /></span>
              <span>{label}</span>
            </div>
          ))}
        </div>
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
    </div>
  );
}

function SettingsHeader({ title, subtitle }) {
  return (
    <div style={{ padding: '20px 28px 16px', borderBottom: '1px solid var(--line)' }}>
      <div className="t-h2" style={{ fontSize: 20 }}>{title}</div>
      {subtitle && <div style={{ fontSize: 13, color: 'var(--ink-3)', marginTop: 3 }}>{subtitle}</div>}
    </div>
  );
}

function SettingsRow({ label, hint, children, danger }) {
  return (
    <div style={{
      display: 'grid', gridTemplateColumns: '240px 1fr',
      gap: 20, padding: '16px 28px', borderBottom: '1px solid var(--line)',
    }}>
      <div>
        <div style={{ fontSize: 13, fontWeight: 500, color: danger ? 'var(--red)' : 'var(--ink)' }}>{label}</div>
        {hint && <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 3, lineHeight: 1.5 }}>{hint}</div>}
      </div>
      <div>{children}</div>
    </div>
  );
}

// ─── Profile ─────────────────────────────────────
function HiSettingsProfile() {
  return (
    <SettingsShell activeSection="profile">
      <SettingsHeader title="Profile" subtitle="Only you and people you share with see this. Server stores an encrypted blob." />
      <SettingsRow label="Display name" hint="Shown on shared links if you choose to reveal it">
        <div className="bb-input" style={{ maxWidth: 340 }}>
          <input defaultValue="Isa Marchetti" />
        </div>
      </SettingsRow>
      <SettingsRow label="Handle" hint="Used for team invites · @isa.marchetti">
        <div className="bb-input" style={{ maxWidth: 340, fontFamily: 'var(--font-mono)' }}>
          <span style={{ color: 'var(--ink-4)' }}>@</span>
          <input defaultValue="isa.marchetti" style={{ fontFamily: 'inherit' }} />
        </div>
      </SettingsRow>
      <SettingsRow label="Avatar" hint="Stored encrypted. Shown only when you choose.">
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <div style={{
            width: 56, height: 56, borderRadius: '50%',
            background: 'linear-gradient(135deg, oklch(0.8 0.15 82), oklch(0.6 0.14 50))',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'var(--paper)', fontSize: 22, fontWeight: 600,
          }}>IM</div>
          <BBBtn size="sm">Upload</BBBtn>
          <BBBtn size="sm" variant="ghost">Remove</BBBtn>
        </div>
      </SettingsRow>
      <SettingsRow label="Public profile" hint="Let people find you by handle when sharing">
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <BBToggle />
          <span style={{ fontSize: 12.5, color: 'var(--ink-3)' }}>Off · you are invisible by handle</span>
        </div>
      </SettingsRow>
      <SettingsRow label="Recovery contact" hint="Optional. Notified (not given access) if your account is inactive for 180 days.">
        <div className="bb-input" style={{ maxWidth: 340 }}>
          <input placeholder="email@example.com" />
        </div>
      </SettingsRow>
    </SettingsShell>
  );
}

// ─── Devices ─────────────────────────────────────
function HiSettingsDevices() {
  const devs = [
    { name: 'MacBook Pro · Safari', plat: 'macOS 14.4', first: '24 Aug 2024', last: 'Active now', current: true, synced: '23.4 GB' },
    { name: 'iPhone 15 Pro', plat: 'iOS 17.5 · v1.2.0', first: '24 Aug 2024', last: '14 min ago', synced: '8.2 GB · Camera only' },
    { name: 'Pixel 8', plat: 'Android 14 · v1.2.0', first: '3 Sep 2024', last: '3 days ago', synced: '4.1 GB · Selective' },
    { name: 'Desktop · Windows', plat: 'Windows 11 · Sync v0.9', first: '12 Oct 2024', last: '6 days ago', synced: '23.4 GB · Full' },
    { name: 'bb CLI', plat: 'Linux · v0.4.1', first: '4 Mar 2026', last: '12 days ago', synced: 'on-demand' },
  ];
  return (
    <SettingsShell activeSection="devices">
      <SettingsHeader title="Devices" subtitle="Every device holding a copy of your vault key. Revoke to re-encrypt your data against a new key." />
      <div style={{ padding: '8px 0' }}>
        {devs.map((d, i, arr) => (
          <div key={i} style={{
            padding: '14px 28px', display: 'flex', alignItems: 'center', gap: 14,
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
          }}>
            <div style={{
              width: 40, height: 40, borderRadius: 8, background: 'var(--paper-2)',
              border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Ico name={d.name.includes('iPhone') || d.name.includes('Pixel') ? 'image' : 'cloud'} size={15} color="var(--ink-2)" />
            </div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13.5, fontWeight: 500, display: 'flex', gap: 8, alignItems: 'center' }}>
                {d.name}
                {d.current && <BBChip variant="green" style={{ fontSize: 9.5 }}><span className="dot" /> This device</BBChip>}
              </div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>
                {d.plat} · first seen {d.first} · {d.last}
              </div>
            </div>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{d.synced}</span>
            {!d.current && <BBBtn size="sm" variant="ghost">Revoke</BBBtn>}
          </div>
        ))}
      </div>
    </SettingsShell>
  );
}

// ─── Notifications ─────────────────────────────────────
function HiSettingsNotifications() {
  const groups = [
    {
      title: 'Security',
      hint: 'We can\'t see your files but we know when devices sign in.',
      items: [
        ['New device signs in', true, true],
        ['Recovery phrase used', true, true],
        ['Password changed', true, true],
        ['Sign-in from new country', true, false],
      ],
    },
    {
      title: 'Sharing',
      hint: 'Activity on links and folders you share.',
      items: [
        ['Someone opens a link you shared', true, false],
        ['Link expires or is revoked', false, false],
        ['New team-vault member joins', true, false],
      ],
    },
    {
      title: 'System',
      hint: null,
      items: [
        ['Storage near limit (>90%)', true, false],
        ['Sub-processor changes', false, true],
        ['Product updates & changelog', false, false],
      ],
    },
  ];
  return (
    <SettingsShell activeSection="notifications">
      <SettingsHeader title="Notifications" subtitle="Choose how we tell you about events. Email is always PGP-signed." />
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 100px 100px',
        padding: '12px 28px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)',
      }}>
        <span className="t-label">Event</span>
        <span className="t-label" style={{ textAlign: 'center' }}>In-app</span>
        <span className="t-label" style={{ textAlign: 'center' }}>Email</span>
      </div>
      {groups.map((g, gi) => (
        <div key={gi}>
          <div style={{ padding: '14px 28px 8px', background: 'var(--paper)' }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--ink-2)' }}>{g.title}</div>
            {g.hint && <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 2 }}>{g.hint}</div>}
          </div>
          {g.items.map(([label, inApp, email], i) => (
            <div key={i} style={{
              display: 'grid', gridTemplateColumns: '1fr 100px 100px',
              padding: '10px 28px', alignItems: 'center',
              borderBottom: '1px solid var(--line)',
            }}>
              <span style={{ fontSize: 13 }}>{label}</span>
              <div style={{ display: 'flex', justifyContent: 'center' }}><BBToggle on={inApp} /></div>
              <div style={{ display: 'flex', justifyContent: 'center' }}><BBToggle on={email} /></div>
            </div>
          ))}
        </div>
      ))}
    </SettingsShell>
  );
}

// ─── Language & Region ─────────────────────────────────────
function HiSettingsLanguage() {
  const langs = [
    ['English', 'en', true, 'complete'],
    ['Nederlands', 'nl', false, 'complete'],
    ['Deutsch', 'de', false, 'complete'],
    ['Français', 'fr', false, 'complete'],
    ['Italiano', 'it', false, '94%'],
    ['Polski', 'pl', false, '82%'],
    ['Español', 'es', false, '72%'],
    ['Svenska', 'sv', false, '51%'],
  ];
  return (
    <SettingsShell activeSection="language">
      <SettingsHeader title="Language & region" />
      <SettingsRow label="Language" hint="We contribute translations upstream — open-source, not machine.">
        <div style={{
          display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 6,
          maxWidth: 520,
        }}>
          {langs.map(([name, code, active, pct], i) => (
            <div key={i} style={{
              padding: '10px 12px', borderRadius: 'var(--r-2)',
              border: '1px solid', borderColor: active ? 'var(--amber-deep)' : 'var(--line)',
              background: active ? 'var(--amber-bg)' : 'var(--paper)',
              display: 'flex', alignItems: 'center', gap: 10, cursor: 'pointer',
            }}>
              <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', width: 20 }}>{code}</span>
              <span style={{ fontSize: 13, fontWeight: active ? 600 : 400, flex: 1 }}>{name}</span>
              <span className="t-mono" style={{ fontSize: 10, color: pct === 'complete' ? 'oklch(0.5 0.1 155)' : 'var(--ink-3)' }}>
                {pct === 'complete' ? '✓' : pct}
              </span>
            </div>
          ))}
        </div>
      </SettingsRow>
      <SettingsRow label="Region format" hint="Affects dates, numbers, currency displays">
        <div className="bb-input" style={{ maxWidth: 280 }}>
          <input defaultValue="Europe · 24h · metric · €" />
          <Ico name="chevDown" size={12} color="var(--ink-3)" />
        </div>
      </SettingsRow>
      <SettingsRow label="Timezone" hint="Used for activity log and shared link expiry">
        <div className="bb-input" style={{ maxWidth: 280 }}>
          <input defaultValue="Europe/Berlin · CEST" />
          <Ico name="chevDown" size={12} color="var(--ink-3)" />
        </div>
      </SettingsRow>
      <SettingsRow label="First day of week">
        <div style={{ display: 'flex', gap: 6 }}>
          {['Sun', 'Mon'].map((d, i) => (
            <div key={i} style={{
              padding: '6px 14px', fontSize: 12.5, borderRadius: 'var(--r-1)',
              background: i === 1 ? 'var(--ink)' : 'var(--paper)',
              color: i === 1 ? 'var(--paper)' : 'var(--ink-2)',
              border: '1px solid', borderColor: i === 1 ? 'var(--ink)' : 'var(--line-2)',
              cursor: 'pointer',
            }}>{d}</div>
          ))}
        </div>
      </SettingsRow>
    </SettingsShell>
  );
}

// ─── Shared with me ─────────────────────────────────────
function HiSharedWithMe() {
  const items = [
    { ic: 'folder', name: 'editorial-review/', from: 'anika@publication.eu', when: '2h ago', expires: 'Never', access: 'Can edit', items: '4 files' },
    { ic: 'file', name: 'fact-check-notes.pdf', from: 'anika@publication.eu', when: 'yesterday', expires: 'in 5 days', access: 'Read only', items: '2.4 MB' },
    { ic: 'folder', name: 'source-materials/', from: 'legal@foundation.eu', when: '3 days ago', expires: 'in 22 days', access: 'Read only', items: '128 files' },
    { ic: 'file', name: 'leaked-contract-draft.pdf', from: 'confidential-link', when: '1 week ago', expires: 'in 47h', access: 'One-time view', items: '1.1 MB', anon: true },
    { ic: 'folder', name: 'podcast-raw/', from: 'pieter@producer.nl', when: '2 weeks ago', expires: 'Never', access: 'Can edit', items: '47 items' },
  ];
  return (
    <div style={{ width: 900 }}>
      <div className="bb-card" style={{ padding: 0 }}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <Ico name="share" size={14} color="var(--amber-deep)" />
          <div>
            <div className="t-h3" style={{ fontSize: 14 }}>Shared with me</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Their key granted you — they can still revoke</div>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
            <BBChip variant="filled" style={{ fontSize: 10.5 }}>All · 5</BBChip>
            <BBChip style={{ fontSize: 10.5 }}>People · 4</BBChip>
            <BBChip style={{ fontSize: 10.5 }}>Anonymous links · 1</BBChip>
          </div>
        </div>
        <div style={{
          display: 'grid', gridTemplateColumns: '32px 1.4fr 1fr 140px 120px 100px 80px',
          gap: 14, padding: '10px 18px', borderBottom: '1px solid var(--line)', background: 'var(--paper-2)',
        }}>
          <div />
          <span className="t-label">Name</span>
          <span className="t-label">From</span>
          <span className="t-label">Access</span>
          <span className="t-label">Expires</span>
          <span className="t-label">Size</span>
          <span />
        </div>
        {items.map((it, i, arr) => (
          <div key={i} style={{
            display: 'grid', gridTemplateColumns: '32px 1.4fr 1fr 140px 120px 100px 80px',
            gap: 14, padding: '11px 18px', alignItems: 'center',
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
          }}>
            <Ico name={it.ic} size={14} color={it.ic === 'folder' ? 'var(--amber-deep)' : 'var(--ink-2)'} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <span style={{ fontSize: 13, fontWeight: 500 }}>{it.name}</span>
              <span className="t-micro" style={{ color: 'var(--ink-4)' }}>{it.when}</span>
            </div>
            <span className="t-mono" style={{ fontSize: 11.5, color: it.anon ? 'var(--ink-3)' : 'var(--ink-2)' }}>
              {it.anon ? <em>{it.from}</em> : it.from}
            </span>
            <BBChip style={{ fontSize: 10, ...(it.access === 'One-time view' ? { color: 'var(--amber-deep)', borderColor: 'oklch(0.86 0.07 90)', background: 'var(--amber-bg)' } : {}) }}>{it.access}</BBChip>
            <span className="t-mono" style={{ fontSize: 11, color: it.expires.includes('47h') ? 'var(--red)' : 'var(--ink-3)' }}>{it.expires}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.items}</span>
            <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <Ico name="more" size={13} color="var(--ink-3)" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Activity feed ─────────────────────────────────────
function HiActivity() {
  const days = [
    {
      date: 'Today',
      items: [
        { ic: 'file', dot: 'oklch(0.72 0.16 155)', what: 'You saved', subject: 'story-draft.md', details: 'v47 · +2 paragraphs', where: 'investigations/ledger-gap', when: '4 min ago' },
        { ic: 'share', dot: 'var(--amber)', what: 'Anika opened', subject: 'fact-check-notes.pdf', details: 'viewed once · from Helsinki', where: 'shared link', when: '2h ago' },
        { ic: 'lock', dot: 'var(--ink)', what: 'You revoked', subject: 'expired session', details: 'Pixel 8 · Warsaw', where: 'sessions', when: '3h ago' },
        { ic: 'upload', dot: 'oklch(0.72 0.16 155)', what: 'Beebeeb received', subject: '42 photos', details: '168 MB · auto-backup from iPhone', where: 'photos/2025-09', when: '6h ago' },
      ],
    },
    {
      date: 'Yesterday',
      items: [
        { ic: 'key', dot: 'var(--amber)', what: 'You rotated', subject: 'vault key', details: '128 files re-encrypted · everything still works', where: 'security', when: '23:14' },
        { ic: 'users', dot: 'var(--ink)', what: 'pieter@producer.nl invited you', subject: 'podcast-raw/', details: 'accepted · 47 items synced', where: 'shared with me', when: '14:22' },
        { ic: 'trash', dot: 'var(--ink-3)', what: 'You deleted', subject: 'old-drafts/ (12 files)', details: 'shreds in 28 days', where: 'drafts/', when: '09:41' },
      ],
    },
    {
      date: 'Monday',
      items: [
        { ic: 'shield', dot: 'oklch(0.72 0.16 155)', what: 'New device signed in', subject: 'MacBook Pro · Safari', details: 'Berlin, Germany · you confirmed', where: 'devices', when: '08:12' },
        { ic: 'file', dot: 'oklch(0.72 0.16 155)', what: 'You restored', subject: 'v42', details: 'of source-B-transcript.pdf', where: 'trash → sources/', when: '07:55' },
      ],
    },
  ];
  return (
    <div style={{ width: 780, background: 'var(--paper)', borderRadius: 'var(--r-3)', border: '1px solid var(--line)', overflow: 'hidden', boxShadow: 'var(--shadow-1)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="clock" size={14} />
        <div>
          <div className="t-h3" style={{ fontSize: 14 }}>Activity</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Events are encrypted per-entry · only you can read them</div>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <BBChip style={{ fontSize: 10.5 }}>Everything</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Just me</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Shares</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Security</BBChip>
        </div>
      </div>
      {days.map((d, di) => (
        <div key={di}>
          <div style={{ padding: '10px 20px 6px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)' }}>
            <span className="t-label" style={{ fontSize: 10 }}>{d.date}</span>
          </div>
          {d.items.map((it, i, arr) => (
            <div key={i} style={{
              padding: '12px 20px', display: 'flex', alignItems: 'flex-start', gap: 14,
              borderBottom: i < arr.length - 1 || di < days.length - 1 ? '1px solid var(--line)' : 'none',
            }}>
              <div style={{
                width: 28, height: 28, borderRadius: '50%',
                background: 'var(--paper-2)', border: '1px solid var(--line)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                flexShrink: 0,
                position: 'relative',
              }}>
                <Ico name={it.ic} size={13} color="var(--ink-2)" />
                <span style={{ position: 'absolute', top: -1, right: -1, width: 7, height: 7, borderRadius: '50%', background: it.dot, border: '1.5px solid var(--paper)' }} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, color: 'var(--ink-2)' }}>
                  <span style={{ color: 'var(--ink)' }}>{it.what}</span>{' '}
                  <span className="t-mono" style={{ fontSize: 12.5, fontWeight: 500, color: 'var(--ink)' }}>{it.subject}</span>
                </div>
                <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>
                  {it.details} · <span style={{ color: 'var(--amber-deep)' }}>{it.where}</span>
                </div>
              </div>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-4)', flexShrink: 0 }}>{it.when}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

Object.assign(window, {
  HiSettingsProfile, HiSettingsDevices, HiSettingsNotifications, HiSettingsLanguage,
  HiSharedWithMe, HiActivity,
});
