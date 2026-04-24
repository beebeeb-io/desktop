// hifi-enterprise.jsx — Enterprise surfaces: audit log, SSO, data export, tokens, IP allowlist, DPA, bulk import, impersonate, billing roles

function HiAuditLog() {
  const rows = [
    { t: '14:32:09', a: 'isa@example.eu', ev: 'file.share.create', tgt: 'evidence/Tirana-minutes.pdf', ip: '85.144.22.10', dev: 'macOS · Safari' },
    { t: '14:28:51', a: 'marc@example.eu', ev: 'member.role.change', tgt: 'legal@example.com → Can view', ip: '91.203.11.4', dev: 'iOS · 17.4' },
    { t: '14:21:03', a: 'isa@example.eu', ev: 'key.rotate', tgt: 'team key · v23', ip: '85.144.22.10', dev: 'macOS · Safari' },
    { t: '13:55:42', a: 'system', ev: 'session.expire', tgt: 'editor@example.eu · Firefox', ip: '—', dev: 'server' },
    { t: '13:12:08', a: 'editor@example.eu', ev: 'file.download', tgt: 'raw/interview-03.wav', ip: '77.22.8.91', dev: 'Windows · Chrome' },
    { t: '12:47:18', a: 'marc@example.eu', ev: 'member.invite', tgt: 'legal@example.com', ip: '91.203.11.4', dev: 'macOS · Chrome' },
    { t: '11:02:30', a: 'isa@example.eu', ev: 'file.upload', tgt: '14 files · 312 MB', ip: '85.144.22.10', dev: 'macOS · Safari' },
    { t: '10:58:14', a: 'isa@example.eu', ev: 'auth.login', tgt: 'passkey', ip: '85.144.22.10', dev: 'macOS · Safari' },
  ];
  return (
    <div style={{ width: 1080, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="shield" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Audit log</div>
        <BBChip variant="green" style={{ marginLeft: 10 }}>Live · signed · append-only</BBChip>
        <span style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <BBBtn size="sm" variant="ghost"><Ico name="download" size={11} /> Export CSV</BBBtn>
          <BBBtn size="sm" variant="ghost"><Ico name="download" size={11} /> Export signed JSON</BBBtn>
        </span>
      </div>
      <div style={{ padding: '10px 18px', display: 'flex', gap: 8, borderBottom: '1px solid var(--line)', background: 'var(--paper-2)', fontSize: 12 }}>
        <input placeholder="Filter by actor, event, target…" style={{ flex: 1, border: '1px solid var(--line-2)', borderRadius: 6, padding: '5px 9px', fontSize: 12, fontFamily: 'var(--font-mono)', background: 'var(--paper)' }} defaultValue="actor:marc@* event:member.*" />
        <BBChip>Last 24h</BBChip>
        <BBChip variant="amber">8 events</BBChip>
      </div>
      <div>
        <div style={{ display: 'grid', gridTemplateColumns: '110px 200px 180px 1fr 140px 160px', padding: '8px 18px', fontSize: 10, fontFamily: 'var(--font-mono)', textTransform: 'uppercase', letterSpacing: 0.06, color: 'var(--ink-3)', borderBottom: '1px solid var(--line)' }}>
          <span>Time</span><span>Actor</span><span>Event</span><span>Target</span><span>IP</span><span>Device</span>
        </div>
        {rows.map((r, i) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '110px 200px 180px 1fr 140px 160px', padding: '9px 18px', fontSize: 12, borderBottom: '1px solid var(--line)', alignItems: 'center' }}>
            <span className="t-mono" style={{ color: 'var(--ink-3)' }}>{r.t}</span>
            <span className="t-mono" style={{ fontSize: 11 }}>{r.a}</span>
            <span className="t-mono" style={{ fontSize: 11, color: r.ev.startsWith('auth') ? 'var(--amber-deep)' : r.ev.startsWith('key') ? 'oklch(0.55 0.15 30)' : 'var(--ink-2)' }}>{r.ev}</span>
            <span style={{ color: 'var(--ink-2)' }}>{r.tgt}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r.ip}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r.dev}</span>
          </div>
        ))}
      </div>
      <div style={{ padding: '10px 18px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', fontSize: 11, color: 'var(--ink-3)', display: 'flex', gap: 8, alignItems: 'center' }}>
        <Ico name="lock" size={11} color="var(--ink-3)" />
        <span className="t-mono">Log tip hash · 7d9a…42c3 · verified</span>
        <span style={{ marginLeft: 'auto' }}>Retention: 13 months · exported signatures valid forever</span>
      </div>
    </div>
  );
}

function HiSSOSetup() {
  return (
    <div style={{ width: 820, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="key" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>SSO · SAML 2.0</div>
        <BBChip variant="amber" style={{ marginLeft: 10 }}>Business tier</BBChip>
        <BBChip variant="green" style={{ marginLeft: 'auto' }}><span className="dot" /> Configured</BBChip>
      </div>
      <div style={{ padding: 22, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 24 }}>
        <div>
          <div className="t-label" style={{ marginBottom: 8 }}>1 · Your SAML provider</div>
          <div style={{ display: 'flex', gap: 8, marginBottom: 14 }}>
            {['Okta', 'Entra ID', 'JumpCloud', 'Generic'].map((p, i) => (
              <div key={p} style={{ padding: '6px 10px', fontSize: 11.5, borderRadius: 6, background: i === 0 ? 'var(--amber-bg)' : 'var(--paper-2)', border: `1px solid ${i === 0 ? 'var(--amber-deep)' : 'var(--line-2)'}` }}>{p}</div>
            ))}
          </div>
          <div className="t-label" style={{ marginBottom: 8 }}>2 · IdP metadata URL</div>
          <div style={{ padding: '8px 10px', borderRadius: 6, border: '1px solid var(--line-2)', background: 'var(--paper-2)', fontSize: 11.5, fontFamily: 'var(--font-mono)', color: 'var(--ink-2)', wordBreak: 'break-all' }}>
            https://panorama.okta.com/app/exk1a2b3/sso/saml/metadata
          </div>
          <div style={{ marginTop: 14 }}>
            <div className="t-label" style={{ marginBottom: 8 }}>3 · Domain claim</div>
            <div style={{ padding: '10px 12px', borderRadius: 8, border: '1px solid oklch(0.87 0.08 155)', background: 'oklch(0.96 0.04 155)', display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ width: 18, height: 18, borderRadius: '50%', background: 'oklch(0.72 0.16 155)', color: 'var(--paper)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, fontWeight: 700 }}>✓</span>
              <div>
                <div className="t-mono" style={{ fontSize: 12 }}>example.eu</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>TXT record verified 2d ago</div>
              </div>
            </div>
          </div>
        </div>
        <div>
          <div className="t-label" style={{ marginBottom: 8 }}>Beebeeb → IdP</div>
          <div style={{ padding: 14, borderRadius: 8, background: 'var(--paper-2)', border: '1px solid var(--line)', fontFamily: 'var(--font-mono)', fontSize: 11, lineHeight: 1.7, color: 'var(--ink-2)' }}>
            <div><span style={{ color: 'var(--ink-3)' }}>ACS URL:</span> https://beebeeb.io/sso/panorama/acs</div>
            <div><span style={{ color: 'var(--ink-3)' }}>Entity ID:</span> urn:beebeeb:panorama</div>
            <div><span style={{ color: 'var(--ink-3)' }}>NameID:</span> emailAddress</div>
            <div><span style={{ color: 'var(--ink-3)' }}>Signing cert:</span> SHA-256 · c2f1…8a9e</div>
          </div>
          <div className="t-label" style={{ marginTop: 18, marginBottom: 8 }}>Policy</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {[
              { l: 'Require SSO for example.eu addresses', on: true },
              { l: 'Allow personal Beebeeb for guests', on: true },
              { l: 'Just-in-time provisioning', on: true },
              { l: 'SCIM user sync', on: false },
            ].map((t, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px', background: 'var(--paper-2)', borderRadius: 6, border: '1px solid var(--line)' }}>
                <span className={`bb-toggle ${t.on ? 'on' : ''}`} />
                <span style={{ fontSize: 12.5 }}>{t.l}</span>
              </div>
            ))}
          </div>
          <div style={{ marginTop: 18, padding: 12, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 8, fontSize: 11.5, color: 'var(--ink-2)', lineHeight: 1.5 }}>
            <strong>E2EE note:</strong> SSO authenticates the session. Your vault key is still derived from the recovery phrase — IdP never sees it.
          </div>
        </div>
      </div>
      <div style={{ padding: '12px 20px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <BBBtn variant="ghost">Test login</BBBtn>
        <BBBtn variant="amber">Save configuration</BBBtn>
      </div>
    </div>
  );
}

function HiDataExport() {
  return (
    <div style={{ width: 620, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="download" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Pack my vault</div>
        <BBChip variant="green" style={{ marginLeft: 'auto' }}>GDPR Art. 20</BBChip>
      </div>
      <div style={{ padding: 22 }}>
        <div className="t-body" style={{ color: 'var(--ink-2)', fontSize: 13, lineHeight: 1.6 }}>
          Export everything as a signed archive. Files stay encrypted with your keys — the archive is portable and we can't read it either.
        </div>
        <div style={{ marginTop: 18, padding: 14, borderRadius: 10, background: 'var(--paper-2)', border: '1px solid var(--line)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 12.5 }}>
            <Ico name="folder" size={13} color="var(--amber-deep)" />
            <span style={{ fontWeight: 600 }}>vault-isa-2026-04-23.tar.zst</span>
            <span className="t-mono" style={{ marginLeft: 'auto', color: 'var(--ink-3)' }}>~19.4 GB</span>
          </div>
          <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
            {[
              { l: 'Files (encrypted, original format on decrypt)', sub: '2,834 items', on: true },
              { l: 'Folder structure + metadata', sub: 'JSON manifest', on: true },
              { l: 'Version history', sub: 'last 30 days', on: true },
              { l: 'Shared links (as inactive records)', sub: '14 links', on: false },
              { l: 'Activity log', sub: 'signed CSV', on: true },
            ].map((o, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span className={`bb-check ${o.on ? 'on' : ''}`} />
                <span style={{ fontSize: 12.5 }}>{o.l}</span>
                <span className="t-micro" style={{ marginLeft: 'auto', color: 'var(--ink-3)' }}>{o.sub}</span>
              </div>
            ))}
          </div>
        </div>
        <div style={{ marginTop: 18, padding: 12, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 8, fontSize: 11.5, lineHeight: 1.55, color: 'var(--ink-2)' }}>
          <strong>You'll need your recovery phrase</strong> to decrypt the archive later. Without it, the files cannot be read — not even by us.
        </div>
        <div style={{ marginTop: 18, display: 'flex', alignItems: 'center', gap: 10 }}>
          <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Archive ready in ~8 min · we'll email you</span>
          <BBBtn variant="amber" style={{ marginLeft: 'auto' }}>Start export</BBBtn>
        </div>
      </div>
    </div>
  );
}

function HiApiTokens() {
  const tokens = [
    { n: 'deploy-bot', scope: 'read:files · write:uploads', last: '12 min ago', exp: 'never', on: true },
    { n: 'isa-laptop-rclone', scope: 'read:* · write:*', last: '2h ago', exp: 'May 2026', on: true },
    { n: 'old-ci-token', scope: 'read:files', last: '41 days ago', exp: 'expired', on: false },
  ];
  return (
    <div style={{ width: 880, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="key" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>API tokens & connected apps</div>
        <BBBtn size="sm" variant="amber" style={{ marginLeft: 'auto' }}>+ New token</BBBtn>
      </div>
      <div style={{ padding: '0 8px' }}>
        {tokens.map((t, i) => (
          <div key={i} style={{ padding: '14px 14px', borderBottom: i < tokens.length - 1 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 14 }}>
            <div style={{ width: 36, height: 36, borderRadius: 8, background: t.on ? 'var(--amber-bg)' : 'var(--paper-2)', border: `1px solid ${t.on ? 'var(--amber-deep)' : 'var(--line-2)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Ico name={t.on ? 'key' : 'x'} size={14} color={t.on ? 'var(--amber-deep)' : 'var(--ink-4)'} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>{t.n}</div>
              <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', marginTop: 2 }}>{t.scope}</div>
            </div>
            <div className="t-micro" style={{ width: 120, color: 'var(--ink-3)' }}>Last used {t.last}</div>
            <BBChip variant={t.exp === 'expired' ? 'default' : t.exp === 'never' ? 'amber' : 'default'}>Expires {t.exp}</BBChip>
            <BBBtn size="sm" variant="ghost">Revoke</BBBtn>
          </div>
        ))}
      </div>
      <div style={{ padding: '14px 20px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)' }}>
        <div className="t-label" style={{ marginBottom: 8 }}>Connected apps</div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          {['rclone', 'Zapier', 'n8n', 'Raycast', 'bb CLI'].map(a => (
            <div key={a} style={{ padding: '6px 10px', borderRadius: 6, background: 'var(--paper)', border: '1px solid var(--line-2)', fontSize: 11.5, display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--green)' }} />
              {a}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function HiIPAllowlist() {
  return (
    <div style={{ width: 720, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="shield" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Network restrictions</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>Business tier</BBChip>
      </div>
      <div style={{ padding: 22 }}>
        <div className="t-label" style={{ marginBottom: 8 }}>IP allowlist</div>
        <div style={{ border: '1px solid var(--line-2)', borderRadius: 8, overflow: 'hidden' }}>
          {[
            { cidr: '85.144.22.0/24', l: 'Amsterdam HQ', on: true },
            { cidr: '91.203.11.4/32', l: 'Marc · home office', on: true },
            { cidr: '2a01:4f8:c17::/48', l: 'Berlin datacenter (IPv6)', on: true },
          ].map((r, i) => (
            <div key={i} style={{ padding: '10px 12px', display: 'flex', alignItems: 'center', gap: 10, borderBottom: i < 2 ? '1px solid var(--line)' : 'none', background: i === 0 ? 'var(--paper)' : 'var(--paper)' }}>
              <span className={`bb-toggle ${r.on ? 'on' : ''}`} />
              <span className="t-mono" style={{ fontSize: 12 }}>{r.cidr}</span>
              <span style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>· {r.l}</span>
              <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--ink-3)' }}>Remove</span>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 10, display: 'flex', gap: 8 }}>
          <input placeholder="203.0.113.0/24" style={{ flex: 1, padding: '7px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12, fontFamily: 'var(--font-mono)' }} />
          <input placeholder="Label" style={{ width: 160, padding: '7px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12 }} />
          <BBBtn size="sm">Add</BBBtn>
        </div>
        <div style={{ marginTop: 20 }} className="t-label">Enforcement</div>
        <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 8 }}>
          {[
            { l: 'Block all traffic outside allowlist', on: true },
            { l: 'Allow admin break-glass from 2FA-verified device', on: true },
            { l: 'Alert on blocked attempt', on: true },
          ].map((p, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px', background: 'var(--paper-2)', borderRadius: 6 }}>
              <span className={`bb-toggle ${p.on ? 'on' : ''}`} />
              <span style={{ fontSize: 12.5 }}>{p.l}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function HiDPAFlow() {
  return (
    <div style={{ width: 760, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="file" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Data Processing Agreement</div>
        <BBChip variant="green" style={{ marginLeft: 'auto' }}>Pre-signed by Beebeeb</BBChip>
      </div>
      <div style={{ padding: 22, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20 }}>
        <div>
          <div className="t-label" style={{ marginBottom: 8 }}>Your organization</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <input defaultValue="Acme Studio" style={{ padding: '8px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12 }} />
            <input defaultValue="Herengracht 420, 1017 BZ Amsterdam" style={{ padding: '8px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12 }} />
            <input defaultValue="isa@example.eu · Data Protection Officer" style={{ padding: '8px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12 }} />
          </div>
          <div className="t-label" style={{ marginTop: 18, marginBottom: 8 }}>Sub-processors you accept</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {[
              ['bit.nl · Amsterdam colocation', true],
              ['Hetzner · Frankfurt failover', true],
              ['Stripe Payments Europe', true],
              ['Postmark (EU) · transactional email', true],
            ].map(([l, on], i) => (
              <label key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12 }}>
                <span className={`bb-check ${on ? 'on' : ''}`} />{l}
              </label>
            ))}
          </div>
        </div>
        <div style={{ padding: 16, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)' }}>
          <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 8 }}>Standard Contractual Clauses</div>
          <div className="t-body" style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.55 }}>
            Our DPA incorporates the 2021 EU Standard Contractual Clauses and reflects Article 28 GDPR. It covers:
          </div>
          <ul style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.65, paddingLeft: 18, marginTop: 8 }}>
            <li>Scope of processing & data categories</li>
            <li>Technical & organisational measures (TOMs)</li>
            <li>Sub-processor list with notice period</li>
            <li>International transfer safeguards (EU-only)</li>
            <li>Audit rights & breach notification within 72h</li>
          </ul>
          <BBBtn size="sm" variant="ghost" style={{ marginTop: 12 }}><Ico name="file" size={11} /> Preview DPA PDF</BBBtn>
        </div>
      </div>
      <div style={{ padding: '12px 20px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', alignItems: 'center', gap: 8 }}>
        <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Countersigned version emailed to your DPO within 2 hours</span>
        <BBBtn variant="amber" style={{ marginLeft: 'auto' }}>Sign DPA</BBBtn>
      </div>
    </div>
  );
}

function HiBulkImport() {
  return (
    <div style={{ width: 760, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="users" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Bulk invite · CSV or SCIM</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>14 new · 2 duplicates</BBChip>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16 }}>
          <div style={{ padding: 14, border: '2px dashed var(--amber-deep)', borderRadius: 10, background: 'var(--amber-bg)', textAlign: 'center' }}>
            <Ico name="upload" size={18} color="var(--amber-deep)" />
            <div style={{ fontSize: 12.5, fontWeight: 600, marginTop: 6 }}>Drop CSV here</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>email, role, folders</div>
          </div>
          <div style={{ padding: 14, border: '1px solid var(--line-2)', borderRadius: 10, background: 'var(--paper-2)', textAlign: 'center' }}>
            <Ico name="key" size={18} color="var(--ink-2)" />
            <div style={{ fontSize: 12.5, fontWeight: 600, marginTop: 6 }}>Auto-sync via SCIM</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>Okta, Entra ID, JumpCloud</div>
          </div>
        </div>
        <div className="t-label" style={{ marginBottom: 8 }}>Preview · panorama-users.csv</div>
        <div style={{ border: '1px solid var(--line-2)', borderRadius: 8, overflow: 'hidden', fontSize: 12 }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1.6fr 1fr 1fr 80px', padding: '8px 12px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', fontFamily: 'var(--font-mono)', fontSize: 10.5, textTransform: 'uppercase', letterSpacing: 0.05, color: 'var(--ink-3)' }}>
            <span>Email</span><span>Role</span><span>Folders</span><span>Status</span>
          </div>
          {[
            ['amira@example.eu', 'Member', 'Tirana-story', 'new'],
            ['david@example.eu', 'Admin', 'all', 'new'],
            ['isa@example.eu', 'Admin', 'all', 'dup'],
            ['legal@example.eu', 'Can view', 'Legal-review', 'new'],
          ].map((r, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '1.6fr 1fr 1fr 80px', padding: '9px 12px', borderBottom: i < 3 ? '1px solid var(--line)' : 'none', alignItems: 'center' }}>
              <span className="t-mono" style={{ fontSize: 11 }}>{r[0]}</span>
              <span>{r[1]}</span>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{r[2]}</span>
              <BBChip variant={r[3] === 'dup' ? 'default' : 'green'}>{r[3]}</BBChip>
            </div>
          ))}
        </div>
      </div>
      <div style={{ padding: '12px 20px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 8, alignItems: 'center' }}>
        <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Duplicates skipped · each new person receives one-time invite link</span>
        <BBBtn variant="amber" style={{ marginLeft: 'auto' }}>Send 14 invites</BBBtn>
      </div>
    </div>
  );
}

function HiImpersonate() {
  return (
    <div style={{ width: 540, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--red)', overflow: 'hidden', boxShadow: 'var(--shadow-3)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', background: 'oklch(0.98 0.03 25)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="shield" size={13} color="var(--red)" />
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--red)' }}>Support access · impersonate</div>
      </div>
      <div style={{ padding: 22 }}>
        <div className="t-body" style={{ fontSize: 13, lineHeight: 1.6, color: 'var(--ink-2)' }}>
          Marc Dubois (<span className="t-mono" style={{ fontSize: 12 }}>marc@example.eu</span>) asked support for help on <strong>evidence/Tirana-minutes.pdf</strong>.
        </div>
        <div style={{ marginTop: 14, padding: 14, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)', fontSize: 12.5 }}>
          <div style={{ marginBottom: 10, fontWeight: 600 }}>What you'll see — and what you won't</div>
          <ul style={{ paddingLeft: 18, color: 'var(--ink-2)', lineHeight: 1.65, margin: 0 }}>
            <li>Folder structure, filenames, activity timeline</li>
            <li>Size, type, share links (as inactive records)</li>
            <li style={{ color: 'var(--red)' }}><strong>Not</strong> file contents — they stay encrypted to Marc's key</li>
            <li style={{ color: 'var(--red)' }}><strong>Not</strong> recovery phrase or passkey secrets</li>
          </ul>
        </div>
        <div style={{ marginTop: 14 }}>
          <div className="t-label" style={{ marginBottom: 8 }}>Required</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {['Approval code from Marc (6 digits)', 'Your hardware key tap', 'Reason (logged forever)'].map((l, i) => (
              <input key={i} placeholder={l} defaultValue={i === 0 ? '429 318' : i === 2 ? 'Ticket #7412 · restore deleted version' : ''} style={{ padding: '8px 10px', border: '1px solid var(--line-2)', borderRadius: 6, fontSize: 12, fontFamily: i === 0 ? 'var(--font-mono)' : 'inherit' }} />
            ))}
          </div>
        </div>
        <div style={{ marginTop: 14, padding: 10, background: 'oklch(0.98 0.03 25)', border: '1px solid oklch(0.88 0.08 25)', borderRadius: 8, fontSize: 11.5, color: 'oklch(0.4 0.1 25)', lineHeight: 1.5 }}>
          Session is capped at 20 minutes. A banner will appear across Marc's dashboard for 30 days.
        </div>
        <div style={{ marginTop: 18, display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <BBBtn variant="ghost">Cancel</BBBtn>
          <BBBtn variant="danger">Start 20-min session</BBBtn>
        </div>
      </div>
    </div>
  );
}

function HiBillingRoles() {
  return (
    <div style={{ width: 860, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="users" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Roles · billing vs. admin split</div>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
          {[
            { r: 'Owner', c: 'Isa Marchetti', badge: 'amber', perms: ['Billing', 'Members', 'Security', 'Data'] },
            { r: 'Billing owner', c: 'accounting@example.eu', badge: 'green', perms: ['Billing only'] },
            { r: 'Admin', c: 'Marc Dubois', badge: 'default', perms: ['Members', 'Security', 'Data'] },
            { r: 'Member', c: '3 people', badge: 'default', perms: ['Files only'] },
          ].map((r, i) => (
            <div key={i} style={{ padding: 14, borderRadius: 10, background: r.badge === 'amber' ? 'var(--amber-bg)' : 'var(--paper-2)', border: `1px solid ${r.badge === 'amber' ? 'var(--amber-deep)' : 'var(--line)'}` }}>
              <div style={{ fontSize: 12.5, fontWeight: 600 }}>{r.r}</div>
              <div className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', marginTop: 4 }}>{r.c}</div>
              <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 4 }}>
                {r.perms.map(p => (
                  <span key={p} style={{ fontSize: 11, color: 'var(--ink-2)', display: 'flex', gap: 6, alignItems: 'center' }}>
                    <span style={{ width: 4, height: 4, borderRadius: '50%', background: 'var(--amber-deep)' }} />{p}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 18 }} className="t-label">Matrix</div>
        <div style={{ marginTop: 8, border: '1px solid var(--line-2)', borderRadius: 8, overflow: 'hidden', fontSize: 12 }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1.4fr repeat(4, 1fr)', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', padding: '8px 12px', fontFamily: 'var(--font-mono)', fontSize: 10.5, textTransform: 'uppercase', letterSpacing: 0.05, color: 'var(--ink-3)' }}>
            <span>Capability</span><span>Owner</span><span>Billing</span><span>Admin</span><span>Member</span>
          </div>
          {[
            ['Update payment method', 1, 1, 0, 0],
            ['See invoices', 1, 1, 1, 0],
            ['Invite members', 1, 0, 1, 0],
            ['Rotate team key', 1, 0, 1, 0],
            ['Delete workspace', 1, 0, 0, 0],
            ['Upload / share files', 1, 0, 1, 1],
          ].map((row, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '1.4fr repeat(4, 1fr)', padding: '9px 12px', borderBottom: i < 5 ? '1px solid var(--line)' : 'none', alignItems: 'center' }}>
              <span>{row[0]}</span>
              {row.slice(1).map((v, j) => (
                <span key={j} style={{ color: v ? 'var(--amber-deep)' : 'var(--ink-4)', fontWeight: 600 }}>{v ? '✓' : '—'}</span>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiAuditLog, HiSSOSetup, HiDataExport, HiApiTokens, HiIPAllowlist, HiDPAFlow, HiBulkImport, HiImpersonate, HiBillingRoles });
