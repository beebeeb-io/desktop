// hifi-security.jsx — Account Security Center

function SecurityNavItem({ icon, label, active, badge }) {
  return (
    <div className={'bb-side-item' + (active ? ' active' : '')}>
      <span className="bb-side-icon"><Ico name={icon} size={13} /></span>
      <span style={{ flex: 1 }}>{label}</span>
      {badge && <span className="t-mono" style={{ fontSize: 10, color: active ? 'var(--amber)' : 'var(--ink-4)' }}>{badge}</span>}
    </div>
  );
}

function SecurityScoreRing({ score = 82 }) {
  const r = 28;
  const c = 2 * Math.PI * r;
  const offset = c - (score / 100) * c;
  return (
    <div style={{ position: 'relative', width: 68, height: 68, flexShrink: 0 }}>
      <svg width="68" height="68" viewBox="0 0 68 68">
        <circle cx="34" cy="34" r={r} fill="none" stroke="var(--paper-3)" strokeWidth="5" />
        <circle cx="34" cy="34" r={r} fill="none" stroke="var(--amber)" strokeWidth="5"
          strokeDasharray={c} strokeDashoffset={offset} strokeLinecap="round"
          transform="rotate(-90 34 34)" />
      </svg>
      <div style={{
        position: 'absolute', inset: 0, display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        fontFamily: 'var(--font-mono)', fontSize: 15, fontWeight: 600,
        fontVariantNumeric: 'tabular-nums',
      }}>
        {score}
      </div>
    </div>
  );
}

function HiSecurity() {
  const checklist = [
    { ok: true,  label: 'Recovery phrase saved', hint: 'Last confirmed 14 Apr 2026' },
    { ok: true,  label: 'Strong master password', hint: '24 chars · entropy 142 bits' },
    { ok: true,  label: 'Two-factor authentication', hint: 'Authenticator app' },
    { ok: false, label: 'Add a passkey', hint: 'Replace your password entirely' },
    { ok: false, label: 'Set up a trusted contact', hint: 'Optional recovery helper' },
  ];

  const devices = [
    { name: 'MacBook Pro', os: 'macOS 15.3 · Safari', loc: 'Amsterdam, NL', when: 'Active now', current: true, icon: 'cloud' },
    { name: 'iPhone 15', os: 'iOS 18.4', loc: 'Amsterdam, NL', when: '12 min ago', current: false, icon: 'image' },
    { name: 'Workstation', os: 'Fedora 41 · Desktop app', loc: 'Amsterdam, NL', when: '3 hours ago', current: false, icon: 'settings' },
    { name: 'Old laptop', os: 'Windows 11 · Chrome', loc: 'Berlin, DE', when: 'Apr 2 · 14d ago', current: false, icon: 'file', stale: true },
  ];

  const events = [
    { ico: 'key',    label: 'Recovery phrase viewed', meta: 'From MacBook Pro · Amsterdam', when: '2m ago', tone: 'amber' },
    { ico: 'shield', label: 'New sign-in approved', meta: 'iPhone 15 · passkey', when: '12m ago', tone: 'ink' },
    { ico: 'share',  label: 'Link shared with 3 viewers', meta: 'term-sheet-v3.docx · 24h expiry', when: '1h ago', tone: 'ink' },
    { ico: 'lock',   label: 'Team key rotated', meta: 'Acme Studio · re-encrypted 128 files', when: 'yesterday', tone: 'green' },
    { ico: 'users',  label: 'Member removed', meta: 'jordan@example.eu · access revoked', when: '3d ago', tone: 'red' },
  ];

  return (
    <div className="bb-card elevated" style={{ width: 1040, height: 760, display: 'flex', overflow: 'hidden' }}>
      {/* Sidebar (settings context) */}
      <div style={{ width: 220, borderRight: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '16px 16px 12px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Ico name="settings" size={13} color="var(--ink-3)" />
          <div className="t-h3" style={{ fontSize: 14 }}>Settings</div>
        </div>
        <div style={{ padding: '6px 12px' }}>
          <SecurityNavItem icon="users" label="Account" />
          <SecurityNavItem icon="shield" label="Security" active badge="!" />
          <SecurityNavItem icon="key" label="Passkeys & 2FA" />
          <SecurityNavItem icon="cloud" label="Devices" badge="4" />
          <SecurityNavItem icon="clock" label="Activity" />
          <SecurityNavItem icon="download" label="Data export" />
        </div>
        <div className="bb-divider" style={{ margin: '10px 16px' }} />
        <div style={{ padding: '0 12px' }}>
          <div className="t-label" style={{ padding: '0 6px 8px' }}>Workspace</div>
          <SecurityNavItem icon="folder" label="Storage & plan" />
          <SecurityNavItem icon="users" label="Team" />
          <SecurityNavItem icon="share" label="Sharing defaults" />
        </div>
        <div style={{ marginTop: 'auto', padding: 16, borderTop: '1px solid var(--line)' }}>
          <BBRegionBadge region="Frankfurt" />
        </div>
      </div>

      {/* Main */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
        {/* Header */}
        <div style={{ padding: '14px 24px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div className="t-h2">Security</div>
              <BBChip variant="amber"><Ico name="shield" size={10} /> Zero-knowledge</BBChip>
            </div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>
              What we can see, reset, or recover on your behalf: nothing. Here's what you can do.
            </div>
          </div>
          <div style={{ marginLeft: 'auto' }}>
            <BBBtn size="sm" variant="ghost" icon={<Ico name="download" size={12} />}>Export audit log</BBBtn>
          </div>
        </div>

        <div className="bb-scroll" style={{ flex: 1, padding: '20px 24px 28px', display: 'flex', flexDirection: 'column', gap: 20 }}>

          {/* Score card */}
          <div style={{
            display: 'grid', gridTemplateColumns: 'auto 1fr',
            gap: 18, padding: 18,
            background: 'var(--paper)', border: '1px solid var(--line-2)',
            borderRadius: 'var(--r-3)', alignItems: 'center',
          }}>
            <SecurityScoreRing score={82} />
            <div>
              <div className="t-label" style={{ marginBottom: 2 }}>Security posture</div>
              <div className="t-h3" style={{ marginBottom: 10 }}>Strong — with two quick wins.</div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px 22px' }}>
                {checklist.map((c, i) => (
                  <div key={i} style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
                    <span style={{
                      width: 14, height: 14, borderRadius: 4, flexShrink: 0, marginTop: 1,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      background: c.ok ? 'oklch(0.94 0.06 155)' : 'var(--paper-2)',
                      border: '1px solid',
                      borderColor: c.ok ? 'oklch(0.85 0.09 155)' : 'var(--line-2)',
                      color: c.ok ? 'oklch(0.45 0.12 155)' : 'var(--ink-4)',
                    }}>
                      <Ico name={c.ok ? 'check' : 'plus'} size={9} />
                    </span>
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div style={{ fontSize: 12.5, color: c.ok ? 'var(--ink-2)' : 'var(--ink)', fontWeight: c.ok ? 400 : 500, lineHeight: 1.3 }}>
                        {c.label}
                      </div>
                      <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 1 }}>{c.hint}</div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>

          {/* Recovery phrase + master credentials */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            <div className="bb-card" style={{ padding: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 2 }}>
                <Ico name="key" size={14} color="var(--amber-deep)" />
                <div className="t-h3" style={{ fontSize: 14 }}>Recovery phrase</div>
                <BBChip variant="amber" style={{ marginLeft: 'auto', fontSize: 9.5 }}>12 words · encrypted</BBChip>
              </div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 10, lineHeight: 1.5 }}>
                Stored as ciphertext. Unlocking it decrypts on <em>this device</em> with your master password — we never see the plaintext.
              </div>

              {/* Crypto chain — trust signal */}
              <div style={{
                padding: '8px 10px', marginBottom: 10,
                background: 'var(--paper-2)', border: '1px solid var(--line)',
                borderRadius: 'var(--r-2)',
                display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap',
                fontFamily: 'var(--font-mono)', fontSize: 10.5, color: 'var(--ink-3)',
              }}>
                <span style={{ color: 'var(--ink-4)' }}>password</span>
                <Ico name="chevRight" size={9} color="var(--ink-4)" />
                <span>Argon2id</span>
                <Ico name="chevRight" size={9} color="var(--ink-4)" />
                <span>AES-256-GCM</span>
                <Ico name="chevRight" size={9} color="var(--ink-4)" />
                <span style={{ color: 'var(--amber-deep)', fontWeight: 500 }}>plaintext · in memory only</span>
              </div>

              {/* Re-auth gated reveal — locked state */}
              <div style={{
                padding: '10px 12px', marginBottom: 10,
                background: 'oklch(0.99 0.008 85)', border: '1px dashed var(--line-2)',
                borderRadius: 'var(--r-2)',
                display: 'flex', alignItems: 'center', gap: 10,
              }}>
                <div style={{
                  width: 26, height: 26, borderRadius: 999,
                  background: 'var(--paper-2)', border: '1px solid var(--line)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  color: 'var(--ink-3)', flexShrink: 0,
                }}>
                  <Ico name="lock" size={12} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 12, fontWeight: 500 }}>Locked — re-auth required</div>
                  <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 1 }}>
                    Password or biometric · auto-hides after 30s
                  </div>
                </div>
                <BBBtn size="sm" variant="amber" icon={<Ico name="eye" size={11} />}>Unlock to reveal</BBBtn>
              </div>

              <div style={{ display: 'flex', gap: 8, marginBottom: 10 }}>
                <BBBtn size="sm" icon={<Ico name="download" size={11} />}>Download PDF</BBBtn>
                <BBBtn size="sm" variant="ghost">Regenerate</BBBtn>
              </div>
              <div style={{ fontSize: 11.5, color: 'var(--ink-4)', display: 'flex', alignItems: 'center', gap: 5 }}>
                <Ico name="clock" size={10} /> Last confirmed 14 Apr · 8 days ago
              </div>
            </div>

            <div className="bb-card" style={{ padding: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 2 }}>
                <Ico name="lock" size={14} color="var(--ink-2)" />
                <div className="t-h3" style={{ fontSize: 14 }}>Master password</div>
              </div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 12, lineHeight: 1.5 }}>
                Used to decrypt your key bundle. Argon2id · memory-hard on your device.
              </div>
              <div style={{
                display: 'flex', alignItems: 'center', gap: 8,
                padding: '8px 10px', background: 'var(--paper-2)',
                border: '1px solid var(--line)', borderRadius: 'var(--r-2)', marginBottom: 10,
              }}>
                <span className="t-mono" style={{ fontSize: 12, letterSpacing: '0.15em', flex: 1 }}>
                  ••••••••••••••••••••••••
                </span>
                <BBChip variant="green" style={{ fontSize: 9.5 }}>Strong</BBChip>
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <BBBtn size="sm">Change password</BBBtn>
                <BBBtn size="sm" variant="ghost">Test strength</BBBtn>
              </div>
            </div>
          </div>

          {/* 2FA + Passkeys */}
          <div className="bb-card" style={{ flexShrink: 0 }}>
            <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Ico name="shield" size={14} />
              <div className="t-h3" style={{ fontSize: 14 }}>Sign-in methods</div>
              <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }} icon={<Ico name="plus" size={11} />}>Add method</BBBtn>
            </div>

            {[
              { t: 'Passkey · MacBook Pro', s: 'Added 3 Apr · Platform authenticator', ico: 'key', on: true },
              { t: 'Authenticator app', s: 'Backup codes downloaded', ico: 'clock', on: true },
              { t: 'Hardware key (YubiKey)', s: 'Recommended for admins', ico: 'shield', on: false, cta: true },
              { t: 'SMS', s: 'Not recommended — disabled by policy', ico: 'more', on: false, disabled: true },
            ].map((m, i, arr) => (
              <div key={i} style={{
                padding: '12px 16px',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                display: 'flex', alignItems: 'center', gap: 12,
                opacity: m.disabled ? 0.55 : 1,
              }}>
                <div style={{
                  width: 28, height: 28, borderRadius: 8,
                  background: m.on ? 'var(--amber-bg)' : 'var(--paper-2)',
                  border: '1px solid', borderColor: m.on ? 'oklch(0.86 0.07 90)' : 'var(--line)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  color: m.on ? 'var(--amber-deep)' : 'var(--ink-3)',
                }}>
                  <Ico name={m.ico} size={13} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500 }}>{m.t}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{m.s}</div>
                </div>
                {m.cta ? <BBBtn size="sm">Set up</BBBtn>
                  : m.disabled ? <BBChip>Off</BBChip>
                  : <BBToggle on />}
              </div>
            ))}
          </div>

          {/* Devices */}
          <div className="bb-card" style={{ flexShrink: 0 }}>
            <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Ico name="cloud" size={14} />
              <div className="t-h3" style={{ fontSize: 14 }}>Trusted devices</div>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-4)' }}>4 active</span>
              <BBBtn size="sm" variant="danger" style={{ marginLeft: 'auto' }}>Sign out everywhere</BBBtn>
            </div>
            {devices.map((d, i, arr) => (
              <div key={i} style={{
                padding: '12px 16px',
                borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                display: 'grid', gridTemplateColumns: '28px 1fr 140px 110px 80px', gap: 14,
                alignItems: 'center',
              }}>
                <div style={{
                  width: 28, height: 28, borderRadius: 6,
                  background: d.stale ? 'var(--paper-2)' : d.current ? 'var(--ink)' : 'var(--paper-2)',
                  border: '1px solid', borderColor: d.current ? 'var(--ink)' : 'var(--line)',
                  color: d.current ? 'var(--amber)' : 'var(--ink-3)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <Ico name={d.icon} size={13} />
                </div>
                <div>
                  <div style={{ fontSize: 13, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 6 }}>
                    {d.name}
                    {d.current && <BBChip variant="amber" style={{ fontSize: 9.5 }}>This device</BBChip>}
                    {d.stale && <BBChip style={{ fontSize: 9.5, color: 'var(--red)', borderColor: 'oklch(0.85 0.08 25)' }}>Idle 14d</BBChip>}
                  </div>
                  <div className="t-micro t-mono" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{d.os}</div>
                </div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{d.loc}</div>
                <div className="t-mono" style={{ fontSize: 11, color: d.current ? 'oklch(0.45 0.12 155)' : 'var(--ink-3)' }}>{d.when}</div>
                <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
                  {!d.current && <BBBtn size="sm" variant="danger">Revoke</BBBtn>}
                </div>
              </div>
            ))}
          </div>

          {/* Activity log */}
          <div className="bb-card" style={{ flexShrink: 0 }}>
            <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Ico name="clock" size={14} />
              <div className="t-h3" style={{ fontSize: 14 }}>Recent security events</div>
              <div style={{ marginLeft: 'auto', display: 'flex', gap: 4, padding: 3, background: 'var(--paper-2)', borderRadius: 'var(--r-2)', border: '1px solid var(--line)' }}>
                {['All', 'Sign-ins', 'Sharing', 'Keys'].map((t, i) => (
                  <span key={t} style={{
                    padding: '3px 9px', borderRadius: 4, fontSize: 11.5,
                    background: i === 0 ? 'var(--paper)' : 'transparent',
                    boxShadow: i === 0 ? 'var(--shadow-1)' : 'none',
                    fontWeight: i === 0 ? 600 : 400, color: i === 0 ? 'var(--ink)' : 'var(--ink-3)',
                  }}>{t}</span>
                ))}
              </div>
            </div>
            {events.map((e, i, arr) => {
              const toneBg = {
                amber: 'var(--amber-bg)', ink: 'var(--paper-2)',
                green: 'oklch(0.96 0.04 155)', red: 'oklch(0.97 0.02 25)',
              }[e.tone];
              const toneC = {
                amber: 'var(--amber-deep)', ink: 'var(--ink-2)',
                green: 'oklch(0.45 0.12 155)', red: 'var(--red)',
              }[e.tone];
              return (
                <div key={i} style={{
                  padding: '11px 16px',
                  borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
                  display: 'flex', alignItems: 'center', gap: 12,
                }}>
                  <div style={{
                    width: 24, height: 24, borderRadius: 999,
                    background: toneBg,
                    color: toneC,
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    flexShrink: 0,
                  }}>
                    <Ico name={e.ico} size={12} />
                  </div>
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 13, fontWeight: 500 }}>{e.label}</div>
                    <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{e.meta}</div>
                  </div>
                  <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)' }}>{e.when}</span>
                </div>
              );
            })}
            <div style={{ padding: '10px 16px', textAlign: 'center', borderTop: '1px solid var(--line)', background: 'var(--paper-2)' }}>
              <span style={{ fontSize: 12, color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>
                View full audit log →
              </span>
            </div>
          </div>

          {/* Data export + Danger zone */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            <div className="bb-card" style={{ padding: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
                <Ico name="download" size={14} />
                <div className="t-h3" style={{ fontSize: 14 }}>Export your data</div>
              </div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 12, lineHeight: 1.5 }}>
                Download an encrypted archive of everything. Art. 20 GDPR — your right, our obligation.
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <BBBtn size="sm">Request export</BBBtn>
                <BBBtn size="sm" variant="ghost">Schedule monthly</BBBtn>
              </div>
            </div>

            <div className="bb-card" style={{ padding: 16, borderColor: 'oklch(0.88 0.05 25)', background: 'oklch(0.99 0.008 25)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
                <Ico name="trash" size={14} color="var(--red)" />
                <div className="t-h3" style={{ fontSize: 14, color: 'var(--red)' }}>Danger zone</div>
              </div>
              <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 12, lineHeight: 1.5 }}>
                Delete account shreds keys irrecoverably. Files become mathematically unreadable — not even by us.
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <BBBtn size="sm" variant="danger">Delete account</BBBtn>
                <BBBtn size="sm" variant="ghost">Transfer ownership</BBBtn>
              </div>
            </div>
          </div>

        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiSecurity });
