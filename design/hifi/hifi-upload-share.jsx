// hifi-upload-share.jsx — Upload A+B & Share B hi-fi

function HiUpload() {
  const files = [
    { n: 'quarterly-report.pdf', size: '4.2 MB', p: 100, stage: 'Done', color: 'ink' },
    { n: 'team-photo.heic', size: '8.1 MB', p: 78, stage: 'Uploading', color: 'amber' },
    { n: 'design-spec.fig', size: '12.4 MB', p: 42, stage: 'Encrypting', color: 'amber' },
    { n: 'archive.zip', size: '340 MB', p: 0, stage: 'Queued', color: 'muted' },
  ];
  return (
    <div className="bb-card elevated" style={{ width: 520, fontFamily: 'var(--font-sans)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center' }}>
        <div className="t-h3">Upload</div>
        <BBChip variant="amber" style={{ marginLeft: 'auto' }}><Ico name="lock" size={10} /> E2EE on device</BBChip>
      </div>

      {/* Drop zone */}
      <div style={{ padding: 20 }}>
        <div style={{
          border: '1.5px dashed var(--line-2)', borderRadius: 'var(--r-3)',
          padding: '28px 18px', textAlign: 'center',
          background: 'var(--paper-2)', position: 'relative', overflow: 'hidden',
        }}>
          <div className="bb-honeycomb" style={{ position: 'absolute', inset: 0, opacity: 0.5 }} />
          <div style={{ position: 'relative' }}>
            <div style={{
              width: 44, height: 44, margin: '0 auto 10px',
              background: 'var(--amber)', borderRadius: 12,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              boxShadow: '0 4px 14px -4px oklch(0.82 0.17 84 / 0.5), inset 0 1px 0 rgba(255,255,255,0.4)',
            }}>
              <Ico name="arrowUp" size={20} color="var(--ink)" />
            </div>
            <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 4 }}>Drop files to encrypt</div>
            <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>
              AES-256-GCM · chunked client-side · EU-only transit
            </div>
            <BBBtn size="sm" style={{ marginTop: 12 }}>or browse…</BBBtn>
          </div>
        </div>
      </div>

      <div className="bb-divider" />

      {/* Timeline */}
      <div style={{ padding: '14px 20px 18px' }}>
        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 12 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Uploading 4 files</div>
          <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)', marginLeft: 'auto' }}>
            178 MB / 365 MB · 14 MB/s
          </span>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {files.map((f, i) => (
            <div key={i}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 5 }}>
                <div style={{
                  width: 18, height: 18, flexShrink: 0,
                  background: f.p === 100 ? 'oklch(0.94 0.06 155)' : 'var(--paper-2)',
                  border: '1px solid var(--line)', borderRadius: 5,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  {f.p === 100
                    ? <Ico name="check" size={11} color="oklch(0.45 0.12 155)" />
                    : <Ico name="file" size={10} color="var(--ink-3)" />}
                </div>
                <span style={{ fontSize: 12.5, fontWeight: 500 }}>{f.n}</span>
                <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)' }}>{f.size}</span>
                <span className="t-mono" style={{
                  marginLeft: 'auto', fontSize: 10.5,
                  color: f.p === 100 ? 'oklch(0.45 0.12 155)' : f.color === 'muted' ? 'var(--ink-4)' : 'var(--amber-deep)',
                  fontWeight: 500,
                }}>
                  {f.stage} · {f.p}%
                </span>
              </div>
              <div className={'bb-progress' + (f.p === 100 ? ' dark' : '')}>
                <div style={{ width: `${f.p}%` }} />
              </div>
            </div>
          ))}
        </div>

        <div style={{
          marginTop: 16, padding: '10px 12px',
          background: 'var(--amber-bg)', border: '1px solid oklch(0.88 0.05 92)',
          borderRadius: 'var(--r-2)',
          display: 'flex', alignItems: 'center', gap: 10,
        }}>
          <Ico name="key" size={14} color="var(--amber-deep)" />
          <span style={{ fontSize: 12, color: 'oklch(0.35 0.06 72)' }}>
            Keys stay on your device — never on our servers.
          </span>
        </div>
      </div>
    </div>
  );
}

function HiShare() {
  const perms = [
    ['Can view', true],
    ['Can download', true],
    ['Can edit', false],
    ['Can re-share', false],
  ];
  return (
    <div className="bb-card elevated" style={{ width: 500, fontFamily: 'var(--font-sans)' }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="share" size={14} />
        <div className="t-h3">Send securely</div>
        <BBChip style={{ marginLeft: 'auto' }}>term-sheet-v3.docx · 88 KB</BBChip>
      </div>

      <div style={{ padding: 22 }}>
        {/* Link */}
        <div className="t-label" style={{ marginBottom: 6 }}>Link</div>
        <div className="bb-input" style={{ marginBottom: 16 }}>
          <Ico name="link" size={13} color="var(--ink-3)" />
          <input value="beebeeb.io/s/7fx2p9" readOnly style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }} />
          <BBBtn size="sm" icon={<Ico name="copy" size={11} />}>Copy</BBBtn>
        </div>

        {/* Decryption key — separated */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
          <span className="t-label">Decryption key</span>
          <BBChip variant="amber" style={{ fontSize: 9.5 }}>Send via a different channel</BBChip>
        </div>
        <div className="bb-input" style={{ background: 'var(--amber-bg)', borderColor: 'oklch(0.86 0.07 90)' }}>
          <Ico name="key" size={13} color="var(--amber-deep)" />
          <input value="k-a1b4·c7de·f204·9911·bb88" readOnly style={{ fontFamily: 'var(--font-mono)', fontSize: 12, color: 'oklch(0.35 0.1 72)', fontWeight: 500 }} />
          <BBBtn size="sm" icon={<Ico name="copy" size={11} />}>Copy</BBBtn>
        </div>
        <div style={{ fontSize: 11.5, color: 'var(--ink-3)', marginTop: 8, display: 'flex', alignItems: 'center', gap: 6 }}>
          <Ico name="shield" size={11} color="var(--amber-deep)" />
          Zero-knowledge by default — we never see the key.
        </div>

        <div className="bb-divider" style={{ margin: '18px 0' }} />

        {/* Expiry + max opens */}
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 18 }}>
          <div>
            <div className="t-label" style={{ marginBottom: 6 }}>Expires</div>
            <div className="bb-input" style={{ padding: '7px 10px' }}>
              <Ico name="clock" size={13} color="var(--ink-3)" />
              <span style={{ flex: 1, fontSize: 13 }}>24 hours</span>
              <Ico name="chevDown" size={12} color="var(--ink-4)" />
            </div>
          </div>
          <div>
            <div className="t-label" style={{ marginBottom: 6 }}>Max opens</div>
            <div className="bb-input" style={{ padding: '7px 10px' }}>
              <span style={{ flex: 1, fontSize: 13 }}>3</span>
              <Ico name="chevDown" size={12} color="var(--ink-4)" />
            </div>
          </div>
        </div>

        {/* Permissions */}
        <div className="t-label" style={{ marginBottom: 10 }}>Permissions</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginBottom: 20 }}>
          {perms.map(([label, on], i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', fontSize: 13 }}>
              <Ico name={['eye', 'download', 'file', 'share'][i]} size={13} color="var(--ink-3)" />
              <span style={{ marginLeft: 10, flex: 1 }}>{label}</span>
              <BBToggle on={on} />
            </div>
          ))}
        </div>

        <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }}>
          <Ico name="lock" size={13} /> Generate encrypted link
        </BBBtn>
      </div>
    </div>
  );
}

Object.assign(window, { HiUpload, HiShare });
