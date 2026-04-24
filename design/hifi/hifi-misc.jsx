// hifi-misc.jsx — email change, storage full, plan downgrade, encrypted email notification, Linux native, desktop uninstall, enterprise demo

function HiEmailChange() {
  return (
    <div style={{ width: 560, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="mail" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Change email address</div>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: 12, background: 'var(--paper-2)', borderRadius: 10 }}>
          <div style={{ flex: 1 }}>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Current</div>
            <div className="t-mono" style={{ fontSize: 13, marginTop: 2 }}>isa@example.eu</div>
          </div>
          <span style={{ fontSize: 20, color: 'var(--ink-3)' }}>→</span>
          <div style={{ flex: 1 }}>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>New</div>
            <input defaultValue="isa.marchetti@example.eu" className="t-mono" style={{ fontSize: 13, marginTop: 2, width: '100%', border: 'none', background: 'transparent', outline: 'none' }} />
          </div>
        </div>
        <div style={{ marginTop: 18, padding: 14, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 10 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, marginBottom: 6 }}>This is just a label.</div>
          <div style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.55 }}>Your vault key isn't tied to your email. Changing it won't re-encrypt anything — your recovery phrase stays the same. We'll send a confirmation to both addresses.</div>
        </div>
        <div style={{ marginTop: 18 }}>
          <div className="t-label" style={{ marginBottom: 8 }}>To confirm</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <input placeholder="Code sent to old address" defaultValue="428 917" style={{ padding: '9px 12px', border: '1px solid var(--line-2)', borderRadius: 7, fontSize: 13, fontFamily: 'var(--font-mono)' }} />
            <input placeholder="Code sent to new address" style={{ padding: '9px 12px', border: '1px solid var(--line-2)', borderRadius: 7, fontSize: 13, fontFamily: 'var(--font-mono)', color: 'var(--ink-3)' }} />
            <span className="t-micro" style={{ color: 'var(--ink-3)' }}>Codes expire in 10 min · we'll also require your passkey</span>
          </div>
        </div>
      </div>
      <div style={{ padding: '12px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <BBBtn variant="ghost">Cancel</BBBtn>
        <BBBtn variant="amber">Confirm change</BBBtn>
      </div>
    </div>
  );
}

function HiStorageFull() {
  return (
    <div style={{ width: 680, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--amber-deep)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', background: 'var(--amber-bg)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="shield" size={13} color="var(--amber-deep)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>You're out of space</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>2 TB used · 0 GB left</BBChip>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.6 }}>Uploads are paused. Your existing files stay safe and accessible — nothing gets deleted automatically.</div>
        <div style={{ marginTop: 16, height: 10, borderRadius: 5, background: 'var(--paper-2)', overflow: 'hidden' }}>
          <div style={{ height: '100%', width: '100%', background: 'var(--amber-deep)' }} />
        </div>
        <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 6, fontSize: 11, color: 'var(--ink-3)' }}>
          <span className="t-mono">2.00 TB / 2.00 TB</span><span className="t-mono">100%</span>
        </div>
        <div style={{ marginTop: 22 }}>
          <div className="t-label" style={{ marginBottom: 10 }}>Biggest folders</div>
          {[
            { n: 'Archive 2025', sz: '412 GB', pct: 20 },
            { n: 'raw/interviews', sz: '298 GB', pct: 14 },
            { n: 'Photos / 2024', sz: '184 GB', pct: 9 },
            { n: 'Trash (emptying will free)', sz: '89 GB', pct: 4, trash: true },
          ].map((f, i) => (
            <div key={i} style={{ padding: '8px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
              <Ico name={f.trash ? 'x' : 'folder'} size={13} color={f.trash ? 'var(--red)' : 'var(--amber-deep)'} />
              <span style={{ fontSize: 12.5, flex: 1 }}>{f.n}</span>
              <div style={{ width: 120, height: 4, background: 'var(--paper-2)', borderRadius: 2, overflow: 'hidden' }}>
                <div style={{ height: '100%', width: `${f.pct * 5}%`, background: f.trash ? 'var(--red)' : 'var(--amber-deep)' }} />
              </div>
              <span className="t-mono" style={{ fontSize: 11, width: 60, textAlign: 'right', color: 'var(--ink-3)' }}>{f.sz}</span>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 20, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
          <div style={{ padding: 14, borderRadius: 10, border: '1px solid var(--line-2)', background: 'var(--paper-2)' }}>
            <div style={{ fontSize: 13, fontWeight: 600 }}>Clean up</div>
            <div style={{ fontSize: 11.5, color: 'var(--ink-3)', marginTop: 4, lineHeight: 1.5 }}>Empty trash, review large files</div>
            <BBBtn size="sm" variant="ghost" style={{ marginTop: 10 }}>Start review</BBBtn>
          </div>
          <div style={{ padding: 14, borderRadius: 10, border: '1px solid var(--amber-deep)', background: 'var(--amber-bg)' }}>
            <div style={{ fontSize: 13, fontWeight: 600 }}>Upgrade to 5 TB</div>
            <div style={{ fontSize: 11.5, color: 'var(--ink-3)', marginTop: 4, lineHeight: 1.5 }}>€9.99/mo · pro-rated today</div>
            <BBBtn size="sm" variant="amber" style={{ marginTop: 10 }}>Upgrade</BBBtn>
          </div>
        </div>
      </div>
    </div>
  );
}

function HiPlanDowngrade() {
  return (
    <div style={{ width: 620, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="shield" size={13} color="var(--red)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Downgrade to Free — a heads up</div>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.6 }}>You're using <strong>412 GB</strong>, but Free gives you <strong>20 GB</strong>. Here's exactly what happens — nothing gets deleted without you saying so.</div>
        <div style={{ marginTop: 18, border: '1px solid var(--line-2)', borderRadius: 10, overflow: 'hidden' }}>
          {[
            { d: 'Now', t: 'Your plan ends · grace period starts', kind: 'ok' },
            { d: '30 days', t: 'Pick which 20 GB to keep', kind: 'soft' },
            { d: '60 days', t: 'Everything still readable · just no uploads / shares', kind: 'soft' },
            { d: '90 days', t: 'Over-limit files go read-only · download anytime', kind: 'warn' },
            { d: '180 days', t: 'If still unresolved, we email you twice before archiving', kind: 'danger' },
          ].map((r, i) => (
            <div key={i} style={{ padding: '12px 14px', borderBottom: i < 4 ? '1px solid var(--line)' : 'none', display: 'flex', alignItems: 'center', gap: 12, background: r.kind === 'danger' ? 'oklch(0.98 0.03 25)' : 'var(--paper)' }}>
              <span className="t-mono" style={{ fontSize: 11, width: 70, color: r.kind === 'danger' ? 'var(--red)' : 'var(--ink-3)' }}>{r.d}</span>
              <span style={{ fontSize: 12.5, flex: 1 }}>{r.t}</span>
              <BBChip variant={r.kind === 'danger' ? 'default' : r.kind === 'warn' ? 'amber' : 'green'}>{r.kind === 'ok' ? 'Today' : r.kind === 'soft' ? 'Safe' : r.kind === 'warn' ? 'Read-only' : 'Emails'}</BBChip>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 18, padding: 14, background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 10, fontSize: 12.5, lineHeight: 1.55 }}>
          <strong>We never delete encrypted data in a way we could recover it.</strong> If you want out for good, use <a style={{ color: 'var(--amber-deep)' }}>Pack my vault</a> first.
        </div>
      </div>
      <div style={{ padding: '12px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <BBBtn variant="ghost">Keep my plan</BBBtn>
        <BBBtn variant="ghost">Pack & delete account</BBBtn>
        <BBBtn>Downgrade anyway</BBBtn>
      </div>
    </div>
  );
}

function HiEncryptedEmail() {
  return (
    <div style={{ width: 620, background: 'var(--paper)', borderRadius: 12, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-1)', fontFamily: 'ui-serif, Georgia, serif' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', background: 'var(--paper-2)', fontFamily: 'var(--font-mono)', fontSize: 11.5, color: 'var(--ink-3)' }}>
        <div><strong style={{ color: 'var(--ink-2)' }}>From:</strong> Beebeeb &lt;no-reply@beebeeb.io&gt;</div>
        <div><strong style={{ color: 'var(--ink-2)' }}>To:</strong> isa@example.eu</div>
        <div><strong style={{ color: 'var(--ink-2)' }}>Subject:</strong> Marc shared "Legal-review" with you</div>
      </div>
      <div style={{ padding: 26, fontSize: 14, color: 'var(--ink)', lineHeight: 1.65 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 18, fontFamily: 'var(--font-sans)' }}>
          <BBHex size={14} />
          <span style={{ fontSize: 13, fontWeight: 600, fontFamily: 'Inter Display, Inter, sans-serif', letterSpacing: -0.2 }}>Beebeeb</span>
        </div>
        <p style={{ margin: 0 }}>Hi Isa,</p>
        <p>Marc Dubois shared the folder <strong>Legal-review</strong> with you (8 items). Keys have been exchanged — the contents are readable only by you and the folder's members.</p>
        <div style={{ margin: '20px 0', padding: 16, background: 'var(--paper-2)', borderRadius: 10, border: '1px solid var(--line)', fontFamily: 'var(--font-sans)' }}>
          <div style={{ fontSize: 12, color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.5, fontWeight: 600 }}>What we know</div>
          <ul style={{ margin: '8px 0 0', paddingLeft: 18, fontSize: 12.5, color: 'var(--ink-2)' }}>
            <li>Folder name and size (needed to notify you)</li>
            <li>When it was shared and by whom</li>
          </ul>
          <div style={{ fontSize: 12, color: 'var(--ink-3)', textTransform: 'uppercase', letterSpacing: 0.5, fontWeight: 600, marginTop: 14 }}>What we don't</div>
          <ul style={{ margin: '8px 0 0', paddingLeft: 18, fontSize: 12.5, color: 'var(--ink-2)' }}>
            <li>Any filename inside the folder</li>
            <li>Any contents, ever — encrypted to your key</li>
          </ul>
        </div>
        <p>Open it in Beebeeb when you're ready — no re-authentication required from this device.</p>
        <a style={{ display: 'inline-block', marginTop: 12, padding: '11px 20px', background: 'var(--amber-deep)', color: 'var(--paper)', borderRadius: 8, textDecoration: 'none', fontSize: 13, fontWeight: 600, fontFamily: 'var(--font-sans)' }}>Open Legal-review →</a>
        <p style={{ marginTop: 28, fontSize: 12, color: 'var(--ink-3)', fontStyle: 'italic' }}>
          Didn't expect this? Marc may have shared to the wrong address. You can safely ignore this email — no one can view the contents without your keys.
        </p>
      </div>
      <div style={{ padding: '14px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--font-sans)' }}>
        Beebeeb (Initlabs B.V.) · Kelvinstraat 34A, 6601 HE Wijchen · KvK 95157565 · <a style={{ color: 'var(--amber-deep)' }}>Manage notifications</a> · <a style={{ color: 'var(--amber-deep)' }}>DPA</a>
      </div>
    </div>
  );
}

function HiLinuxTray() {
  return (
    <div style={{ width: 720, height: 460, background: 'oklch(0.22 0.005 280)', borderRadius: 8, overflow: 'hidden', boxShadow: 'var(--shadow-3)', display: 'flex', flexDirection: 'column', position: 'relative' }}>
      {/* GNOME-ish top bar */}
      <div style={{ height: 28, background: 'oklch(0.14 0.005 280)', display: 'flex', alignItems: 'center', padding: '0 14px', gap: 14, fontSize: 11, color: 'oklch(0.85 0.01 280)' }}>
        <span>Activities</span>
        <span style={{ margin: '0 auto' }} className="t-mono">Thu 23 Apr  14:32</span>
        <span style={{ display: 'flex', gap: 8 }}>
          <span style={{ width: 12, height: 12, borderRadius: 2, background: 'var(--amber)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}><span style={{ fontSize: 8, fontWeight: 700, color: '#000' }}>B</span></span>
          <span>▾ ♫ ⚡</span>
        </span>
      </div>
      {/* wallpaper */}
      <div style={{ flex: 1, background: 'linear-gradient(135deg, oklch(0.35 0.06 260), oklch(0.22 0.08 290))', position: 'relative' }}>
        {/* tray menu */}
        <div style={{ position: 'absolute', top: 8, right: 60, width: 320, background: 'oklch(0.17 0.005 280)', borderRadius: 8, border: '1px solid oklch(0.28 0.005 280)', overflow: 'hidden', boxShadow: '0 20px 40px -10px rgba(0,0,0,0.6)' }}>
          <div style={{ padding: '12px 14px', borderBottom: '1px solid oklch(0.26 0.005 280)', display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{ width: 24, height: 24, borderRadius: 6, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <BBHex size={10} />
            </div>
            <div>
              <div style={{ fontSize: 12, fontWeight: 600, color: 'oklch(0.94 0.01 280)' }}>Beebeeb</div>
              <div style={{ fontSize: 10, color: 'oklch(0.65 0.01 280)', marginTop: 1 }}>Up to date · 2,834 files</div>
            </div>
            <span style={{ marginLeft: 'auto', fontSize: 9, padding: '1px 6px', borderRadius: 3, background: 'oklch(0.35 0.08 155)', color: 'oklch(0.92 0.04 155)' }}>● synced</span>
          </div>
          {[
            { i: 'folder', t: 'Open in Files', k: '' },
            { i: 'users', t: 'Shared with me', k: '' },
            { i: 'upload', t: 'Add folder to sync', k: '' },
            { i: 'shield', t: 'Preferences', k: 'Ctrl ,' },
            { i: 'x', t: 'Quit', k: 'Ctrl Q' },
          ].map((r, i) => (
            <div key={i} style={{ padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 10, fontSize: 11.5, color: 'oklch(0.88 0.01 280)', borderBottom: i < 4 ? '1px solid oklch(0.24 0.005 280)' : 'none' }}>
              <span style={{ opacity: 0.7 }}>◆</span>
              <span>{r.t}</span>
              {r.k && <span className="t-mono" style={{ marginLeft: 'auto', fontSize: 10, color: 'oklch(0.55 0.01 280)' }}>{r.k}</span>}
            </div>
          ))}
          <div style={{ padding: '10px 14px', fontSize: 10, color: 'oklch(0.6 0.01 280)', background: 'oklch(0.14 0.005 280)', borderTop: '1px solid oklch(0.26 0.005 280)', fontFamily: 'var(--font-mono)' }}>
            ~/Beebeeb · 23.4 GB · last sync 2 min ago
          </div>
        </div>
      </div>
      {/* taskbar stub */}
      <div style={{ height: 36, background: 'oklch(0.14 0.005 280)', display: 'flex', alignItems: 'center', padding: '0 14px', gap: 10 }}>
        {['Files', 'Terminal', 'Firefox'].map(a => (
          <span key={a} style={{ padding: '3px 8px', borderRadius: 4, background: 'oklch(0.22 0.005 280)', fontSize: 10, color: 'oklch(0.85 0.01 280)' }}>{a}</span>
        ))}
      </div>
    </div>
  );
}

function HiDesktopUninstall() {
  return (
    <div style={{ width: 560, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="x" size={13} color="var(--ink-2)" />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Sign out & uninstall</div>
      </div>
      <div style={{ padding: 22 }}>
        <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.6 }}>Clean up properly before removing the app. Your cloud files aren't affected — we're just wiping local traces.</div>
        <div style={{ marginTop: 18, display: 'flex', flexDirection: 'column', gap: 8 }}>
          {[
            { l: 'Unregister this device (frees 1 of 5 slots)', on: true, req: true },
            { l: 'Delete local cache (23.4 GB)', on: true, req: false },
            { l: 'Remove local keys from keychain', on: true, req: true },
            { l: 'Keep offline-available files readable', on: false, req: false },
            { l: 'Delete sync folder at ~/Beebeeb', on: false, req: false },
          ].map((o, i) => (
            <label key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px', background: 'var(--paper-2)', borderRadius: 6, opacity: o.req ? 0.9 : 1 }}>
              <span className={`bb-check ${o.on ? 'on' : ''}`} />
              <span style={{ fontSize: 12.5, flex: 1 }}>{o.l}</span>
              {o.req && <BBChip>Required</BBChip>}
            </label>
          ))}
        </div>
        <div style={{ marginTop: 18, padding: 14, background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)', borderRadius: 10, fontSize: 12.5, lineHeight: 1.55 }}>
          Your vault key lives only in memory once unlocked. After uninstall, signing in on a new device requires your recovery phrase.
        </div>
      </div>
      <div style={{ padding: '12px 22px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <BBBtn variant="ghost">Cancel</BBBtn>
        <BBBtn variant="danger">Sign out & clean up</BBBtn>
      </div>
    </div>
  );
}

function HiEnterpriseDemo() {
  return (
    <div style={{ width: 820, background: 'var(--paper)', borderRadius: 14, border: '1px solid var(--line-2)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      <div style={{ padding: '16px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <BBHex size={12} />
        <div style={{ fontSize: 14, fontWeight: 600 }}>Book a demo · 30 minutes</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}>Enterprise</BBChip>
      </div>
      <div style={{ padding: 24, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 22 }}>
        <div>
          <div className="t-label" style={{ marginBottom: 10 }}>Pick a time</div>
          <div style={{ padding: 14, border: '1px solid var(--line-2)', borderRadius: 10, background: 'var(--paper-2)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>April 2026</span>
              <span style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
                <span style={{ fontSize: 16, color: 'var(--ink-3)' }}>‹</span>
                <span style={{ fontSize: 16, color: 'var(--ink-2)' }}>›</span>
              </span>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)', gap: 4, fontSize: 10, color: 'var(--ink-3)', textAlign: 'center', marginBottom: 6 }}>
              {['M','T','W','T','F','S','S'].map((d, i) => <span key={i}>{d}</span>)}
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)', gap: 4, fontSize: 11.5 }}>
              {Array.from({length: 30}).map((_, i) => {
                const past = i < 22;
                const today = i === 22;
                const avail = [23, 24, 27, 28, 29].includes(i);
                const sel = i === 24;
                return (
                  <div key={i} style={{ padding: '6px 0', textAlign: 'center', borderRadius: 5, background: sel ? 'var(--amber-deep)' : today ? 'var(--paper)' : avail ? 'var(--amber-bg)' : 'transparent', color: sel ? 'var(--paper)' : past ? 'var(--ink-4)' : 'var(--ink-2)', fontWeight: sel || today ? 600 : 400, border: today ? '1px solid var(--ink-3)' : 'none' }}>{i + 1}</div>
                );
              })}
            </div>
          </div>
          <div className="t-label" style={{ marginTop: 18, marginBottom: 10 }}>Friday, 25 April · CEST</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>
            {['09:00', '10:30', '11:00', '14:00', '15:30', '16:30'].map((t, i) => (
              <div key={t} style={{ padding: '8px 0', textAlign: 'center', fontSize: 12, borderRadius: 6, border: `1px solid ${i === 3 ? 'var(--amber-deep)' : 'var(--line-2)'}`, background: i === 3 ? 'var(--amber-bg)' : 'var(--paper)', fontWeight: i === 3 ? 600 : 400 }}>{t}</div>
            ))}
          </div>
        </div>
        <div>
          <div className="t-label" style={{ marginBottom: 10 }}>You'll meet</div>
          <div style={{ padding: 14, border: '1px solid var(--line-2)', borderRadius: 10, display: 'flex', gap: 12, alignItems: 'center' }}>
            <div style={{ width: 46, height: 46, borderRadius: '50%', background: 'linear-gradient(135deg, var(--amber), var(--amber-deep))', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--paper)', fontWeight: 700, fontSize: 16 }}>B</div>
            <div>
              <div style={{ fontSize: 13.5, fontWeight: 600 }}>Bram & Guus Langelaar</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>CEO & CTO · one of us will do your demo personally</div>
            </div>
          </div>
          <div className="t-label" style={{ marginTop: 18, marginBottom: 10 }}>We'll cover</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 12.5 }}>
            {['Live walkthrough of web, mobile, desktop', 'SSO · audit log · DPA workflow', 'Your specific threat model + compliance questions', 'Pricing for your seat count + storage', 'Migration plan from current provider'].map(l => (
              <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ width: 14, height: 14, borderRadius: '50%', background: 'var(--amber-bg)', color: 'var(--amber-deep)', fontSize: 9, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>✓</span>
                {l}
              </div>
            ))}
          </div>
          <BBBtn variant="amber" size="lg" style={{ marginTop: 20, width: '100%', justifyContent: 'center' }}>Confirm · Fri 14:00 CEST</BBBtn>
          <div className="t-micro" style={{ color: 'var(--ink-3)', textAlign: 'center', marginTop: 8 }}>Calendar invite + video link arrive instantly</div>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { HiEmailChange, HiStorageFull, HiPlanDowngrade, HiEncryptedEmail, HiLinuxTray, HiDesktopUninstall, HiEnterpriseDemo });
