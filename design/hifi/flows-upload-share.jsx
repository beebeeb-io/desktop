// flows-upload-share.jsx — Upload with encryption progress + Link sharing

// ─── Upload A — Encryption timeline, linear ────────────────────────────
function UploadA() {
  return (
    <div className="wf-card" style={{ width: 460, padding: 22, fontFamily: 'var(--sans)' }}>
      <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 14 }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Uploading 4 files</span>
        <Chip amber>⬡ encrypting on your device</Chip>
      </div>

      {[
        { n: 'quarterly-report.pdf', p: 100, stage: 'done' },
        { n: 'team-photo.heic', p: 78, stage: 'uploading' },
        { n: 'design-spec.fig', p: 42, stage: 'encrypting' },
        { n: 'archive.zip', p: 0, stage: 'queued' },
      ].map((f, i) => (
        <div key={i} style={{ marginBottom: 14 }}>
          <div className="wf-row" style={{ justifyContent: 'space-between', fontSize: 12, marginBottom: 4 }}>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 11 }}>{f.n}</span>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--ink-3)' }}>{f.stage} · {f.p}%</span>
          </div>
          <div style={{ height: 6, border: '1px solid var(--ink)', display: 'flex' }}>
            <div style={{ width: `${f.p}%`, background: f.stage === 'done' ? 'var(--ink)' : 'var(--amber)' }} />
          </div>
          {i === 2 && (
            <div style={{ display: 'flex', fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)', marginTop: 3, gap: 10 }}>
              <span>◉ key derived</span><span>◉ chunking</span><span>○ uploading</span>
            </div>
          )}
        </div>
      ))}

      <Rule dashed style={{ margin: '12px 0' }} />
      <div style={{ fontFamily: 'var(--hand)', fontSize: 13, color: 'var(--ink-2)' }}>
        Files are encrypted before they leave your device. Keys stay with you.
      </div>
    </div>
  );
}

// ─── Upload B — Drop zone + active transfers card ──────────────────────
function UploadB() {
  return (
    <div className="wf-card" style={{ width: 460, padding: 0, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div style={{
        border: '2px dashed var(--ink)', margin: 18, padding: '28px 14px', textAlign: 'center',
        background: 'var(--paper-2)',
      }}>
        <HexCluster style={{ margin: '0 auto 8px' }} />
        <div style={{ fontFamily: 'var(--hand)', fontSize: 22, marginBottom: 4 }}>Drop to encrypt</div>
        <div style={{ fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--mono)' }}>
          AES-256-GCM · chunked client-side · EU-only transit
        </div>
      </div>

      <div style={{ padding: '0 18px 18px' }}>
        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', textTransform: 'uppercase', color: 'var(--ink-3)', letterSpacing: 0.6, marginBottom: 8 }}>Active · 3</div>
        {[
          ['quarterly-report.pdf', 100, '#1a1a1a'],
          ['team-photo.heic', 78, '#f5b800'],
          ['design-spec.fig', 42, '#f5b800'],
        ].map(([n, p, c], i) => (
          <div key={i} className="wf-row" style={{ gap: 10, padding: '8px 0', borderBottom: '1px dashed rgba(26,26,26,0.2)' }}>
            <div style={{ width: 16, height: 16, border: '1.1px solid var(--ink)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 9 }}>⬡</div>
            <span style={{ flex: 1, fontFamily: 'var(--mono)', fontSize: 11 }}>{n}</span>
            <div style={{ width: 80, height: 5, border: '1px solid var(--ink)' }}>
              <div style={{ width: `${p}%`, height: '100%', background: c }} />
            </div>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 10, width: 30, textAlign: 'right' }}>{p}%</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Upload C — Detailed inspector with cryptographic steps ────────────
function UploadC() {
  return (
    <div className="wf-card" style={{ width: 460, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 16px', borderBottom: '1.2px solid var(--ink)', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 12, fontWeight: 600 }}>Upload inspector</span>
        <Chip>design-spec.fig · 12 MB</Chip>
      </div>
      <div style={{ padding: 16, fontFamily: 'var(--mono)', fontSize: 11 }}>
        {[
          ['1', 'derive file key (HKDF-SHA256)', 'done'],
          ['2', 'chunk file → 64 × 256 KB', 'done'],
          ['3', 'encrypt chunks (AES-256-GCM)', 'in progress 42%'],
          ['4', 'upload to region: eu-fra-1', 'queued'],
          ['5', 'commit manifest, wrap key', 'queued'],
        ].map(([n, label, s], i) => (
          <div key={i} className="wf-row" style={{ padding: '8px 0', gap: 10, borderBottom: '1px dashed rgba(26,26,26,0.18)' }}>
            <span style={{ color: 'var(--ink-4)', width: 12 }}>{n}</span>
            <span style={{ flex: 1, color: s === 'done' ? 'var(--ink-3)' : 'var(--ink)', textDecoration: s === 'done' ? 'line-through' : 'none' }}>{label}</span>
            <Chip amber={s.includes('progress')} style={{ fontSize: 9 }}>{s}</Chip>
          </div>
        ))}
        <div style={{ marginTop: 10, fontFamily: 'var(--hand)', fontSize: 13, color: 'var(--ink-2)' }}>
          nerd-mode on. a "compact" toggle hides this for regular users.
        </div>
      </div>
    </div>
  );
}

// ─── Share A — Compact link card ───────────────────────────────────────
function ShareA() {
  return (
    <div className="wf-card" style={{ width: 420, padding: 20, fontFamily: 'var(--sans)' }}>
      <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 14 }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Share link</div>
          <div style={{ fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--mono)' }}>term-sheet-v3.docx</div>
        </div>
        <Chip amber>⬡ E2EE</Chip>
      </div>

      <div style={{
        display: 'flex', border: '1.2px solid var(--ink)', padding: 6, marginBottom: 14,
        fontFamily: 'var(--mono)', fontSize: 11, alignItems: 'center',
      }}>
        <span style={{ flex: 1, padding: '4px 6px', color: 'var(--ink)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          beebeeb.io/s/7fx2p9·#k=a1b4c7…
        </span>
        <Btn small>Copy</Btn>
      </div>

      <div style={{ display: 'grid', gap: 10 }}>
        <Field label="Password" value="••••••••" hint="required to open" />
        <Field label="Expires" value="in 7 days" />
        <Field label="Max downloads" value="Unlimited" />
      </div>

      <Rule dashed style={{ margin: '14px 0 10px' }} />
      <label style={{ display: 'flex', gap: 8, fontSize: 11, color: 'var(--ink-2)' }}>
        <span style={{ width: 13, height: 13, border: '1.2px solid var(--ink)', flexShrink: 0 }} />
        Notify me when someone opens it
      </label>
    </div>
  );
}

// ─── Share B — Decryption key separate from URL (visual trust) ─────────
function ShareB() {
  return (
    <div className="wf-card" style={{ width: 460, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      <div className="wf-row" style={{ padding: '12px 18px', borderBottom: '1.2px solid var(--ink)' }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Send securely</span>
        <Chip style={{ marginLeft: 'auto' }}>term-sheet-v3.docx · 88 KB</Chip>
      </div>

      <div style={{ padding: 18 }}>
        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', marginBottom: 6, letterSpacing: 0.6 }}>Link</div>
        <div style={{ border: '1.2px solid var(--ink)', padding: 8, fontFamily: 'var(--mono)', fontSize: 11, marginBottom: 12, background: 'var(--paper-2)' }}>
          beebeeb.io/s/7fx2p9
        </div>

        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', marginBottom: 6, letterSpacing: 0.6 }}>Decryption key <span style={{ color: 'var(--amber-deep)' }}>· send through a different channel</span></div>
        <div style={{ border: '1.2px solid var(--ink)', padding: 8, fontFamily: 'var(--mono)', fontSize: 11, marginBottom: 6, background: 'var(--amber-soft)' }}>
          k-a1b4·c7de·f204·9911·bb88
        </div>
        <div className="wf-annot" style={{ marginBottom: 16, fontSize: 13 }}>
          ↑ email the link, message the key. zero-knowledge by default.
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 12 }}>
          <Field label="Expires" value="24 hours" />
          <Field label="Max opens" value="3" />
        </div>

        <div style={{ fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--ink-3)', textTransform: 'uppercase', marginBottom: 8, letterSpacing: 0.6 }}>Permissions</div>
        <div style={{ display: 'grid', gap: 6, marginBottom: 14 }}>
          {[
            ['Can view', true],
            ['Can download', true],
            ['Can edit', false],
            ['Can re-share', false],
          ].map(([l, on], i) => (
            <div key={i} className="wf-row" style={{ justifyContent: 'space-between', fontSize: 12 }}>
              <span>{l}</span>
              <span style={{
                width: 26, height: 14, border: '1px solid var(--ink)',
                background: on ? 'var(--amber)' : 'var(--paper-2)',
                position: 'relative',
              }}>
                <span style={{
                  position: 'absolute', top: 1, [on ? 'right' : 'left']: 1,
                  width: 10, height: 10, background: 'var(--ink)',
                }} />
              </span>
            </div>
          ))}
        </div>

        <Btn amber style={{ width: '100%', justifyContent: 'center' }}>Generate link</Btn>
      </div>
    </div>
  );
}

// ─── Share C — Recipient-based (like sending an envelope) ──────────────
function ShareC() {
  return (
    <div className="wf-card" style={{ width: 420, padding: 20, fontFamily: 'var(--sans)' }}>
      <div style={{ fontFamily: 'var(--hand)', fontSize: 20, marginBottom: 4 }}>Send to…</div>
      <div style={{ fontSize: 11, color: 'var(--ink-3)', marginBottom: 14 }}>They'll get a link keyed to their identity.</div>

      <div style={{ border: '1.2px solid var(--ink)', padding: 10, marginBottom: 10, minHeight: 60 }}>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
          {['anna@lawfirm.fr', 'client-portal:a2z'].map((t, i) => (
            <span key={i} style={{
              padding: '3px 8px', background: i === 1 ? 'var(--amber-soft)' : 'var(--paper-2)',
              border: '1px solid var(--ink)', fontFamily: 'var(--mono)', fontSize: 11,
            }}>{t} ✕</span>
          ))}
          <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--ink-4)' }}>|</span>
        </div>
      </div>

      <div className="wf-annot" style={{ marginBottom: 14, fontSize: 13 }}>
        non-account recipients get a client-portal link &nbsp;↓
      </div>

      <div style={{ display: 'grid', gap: 8, marginBottom: 14 }}>
        {[
          ['can view', true],
          ['can download', true],
          ['can re-share', false],
          ['watermark with email', true],
        ].map(([l, on], i) => (
          <div key={i} className="wf-row" style={{ justifyContent: 'space-between', fontSize: 12 }}>
            <span>{l}</span>
            <span style={{
              width: 26, height: 14, border: '1px solid var(--ink)',
              background: on ? 'var(--amber)' : 'var(--paper-2)',
              position: 'relative',
            }}>
              <span style={{
                position: 'absolute', top: 1, [on ? 'right' : 'left']: 1,
                width: 10, height: 10, background: 'var(--ink)',
              }} />
            </span>
          </div>
        ))}
      </div>
      <Btn amber style={{ width: '100%', justifyContent: 'center' }}>Send encrypted</Btn>
    </div>
  );
}

// ─── Upload A+B merged — Drop zone on top, encryption timeline below ──
function UploadAB() {
  return (
    <div className="wf-card" style={{ width: 500, fontFamily: 'var(--sans)', overflow: 'hidden' }}>
      {/* drop zone */}
      <div style={{
        border: '2px dashed var(--ink)', margin: 18, padding: '22px 14px', textAlign: 'center',
        background: 'var(--paper-2)',
      }}>
        <HexCluster style={{ margin: '0 auto 6px' }} />
        <div style={{ fontFamily: 'var(--hand)', fontSize: 20, marginBottom: 4 }}>Drop to encrypt</div>
        <div style={{ fontSize: 10, color: 'var(--ink-3)', fontFamily: 'var(--mono)' }}>
          AES-256-GCM · chunked client-side · EU-only transit
        </div>
      </div>

      {/* timeline below */}
      <div style={{ padding: '0 20px 18px' }}>
        <div className="wf-row" style={{ justifyContent: 'space-between', marginBottom: 10 }}>
          <span style={{ fontSize: 12, fontWeight: 600 }}>Uploading 4 files</span>
          <Chip amber>⬡ encrypting</Chip>
        </div>
        {[
          { n: 'quarterly-report.pdf', p: 100, stage: 'done' },
          { n: 'team-photo.heic', p: 78, stage: 'uploading' },
          { n: 'design-spec.fig', p: 42, stage: 'encrypting' },
          { n: 'archive.zip', p: 0, stage: 'queued' },
        ].map((f, i) => (
          <div key={i} style={{ marginBottom: 10 }}>
            <div className="wf-row" style={{ justifyContent: 'space-between', fontSize: 11, marginBottom: 3 }}>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>{f.n}</span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--ink-3)' }}>{f.stage} · {f.p}%</span>
            </div>
            <div style={{ height: 5, border: '1px solid var(--ink)', display: 'flex' }}>
              <div style={{ width: `${f.p}%`, background: f.stage === 'done' ? 'var(--ink)' : 'var(--amber)' }} />
            </div>
          </div>
        ))}
        <div style={{ fontFamily: 'var(--hand)', fontSize: 12, color: 'var(--ink-2)', marginTop: 10 }}>
          Keys stay with you — never on our servers.
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { UploadA, UploadB, UploadC, UploadAB, ShareA, ShareB, ShareC });
