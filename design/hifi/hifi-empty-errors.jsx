// hifi-empty-errors.jsx — Empty + error states

// ── 1. Empty drive (just signed up) ─────────────────────────────
function HiEmptyDrive() {
  return (
    <div className="bb-card elevated" style={{ width: 880, height: 520, display: 'flex', flexDirection: 'column' }}>
      <div style={{ padding: '12px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <BBLogo size={14} />
        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8 }}>
          <BBRegionBadge />
          <div style={{ width: 28, height: 28, borderRadius: 999, background: '#f5b800', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, fontWeight: 700 }}>A</div>
        </div>
      </div>
      <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 32, position: 'relative' }}>
        <div className="bb-honeycomb" style={{ position: 'absolute', inset: 0, opacity: 0.45 }} />
        <div style={{ position: 'relative', maxWidth: 520, textAlign: 'center' }}>
          <div style={{
            width: 58, height: 58, margin: '0 auto 18px',
            background: 'var(--amber)', borderRadius: 16,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            boxShadow: '0 8px 24px -6px oklch(0.82 0.17 84 / 0.5), inset 0 1px 0 rgba(255,255,255,0.4)',
            transform: 'rotate(-4deg)',
          }}>
            <Ico name="arrowUp" size={24} color="var(--ink)" />
          </div>
          <div className="t-h1" style={{ marginBottom: 8 }}>Welcome, Anna.</div>
          <div style={{ fontSize: 14, color: 'var(--ink-3)', marginBottom: 22, lineHeight: 1.5 }}>
            Your drive is empty and encrypted, waiting for the first file. Keys live only on your devices — start by dragging something in.
          </div>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'center', marginBottom: 24 }}>
            <BBBtn variant="amber" size="lg" icon={<Ico name="upload" size={13} />}>Upload first file</BBBtn>
            <BBBtn size="lg" icon={<Ico name="download" size={13} />}>Install desktop app</BBBtn>
            <BBBtn variant="ghost" size="lg">Take a tour</BBBtn>
          </div>
          <div style={{
            display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10,
            padding: 14, background: 'var(--paper)', border: '1px solid var(--line)',
            borderRadius: 'var(--r-3)', boxShadow: 'var(--shadow-1)',
          }}>
            {[
              ['shield', 'Encrypted', 'Before it leaves your device'],
              ['users', 'Share safely', 'Keys separate from links'],
              ['cloud', 'Stored in EU', 'Frankfurt · Amsterdam · Paris'],
            ].map(([ico, t, s], i) => (
              <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'flex-start', textAlign: 'left' }}>
                <div style={{
                  width: 24, height: 24, borderRadius: 6,
                  background: 'var(--amber-bg)', color: 'var(--amber-deep)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                }}>
                  <Ico name={ico} size={12} />
                </div>
                <div>
                  <div style={{ fontSize: 12, fontWeight: 600 }}>{t}</div>
                  <div className="t-micro" style={{ color: 'var(--ink-3)' }}>{s}</div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

// ── 2. Search — no results ──────────────────────────────────────
function HiEmptySearch() {
  return (
    <div className="bb-card elevated" style={{ width: 560, padding: 0 }}>
      <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--line)' }}>
        <div className="bb-input">
          <Ico name="search" size={13} color="var(--ink-3)" />
          <input value='"nota-acuerdo.pdf"' readOnly style={{ fontFamily: 'var(--font-mono)', fontSize: 12.5 }} />
          <BBKbd>esc</BBKbd>
        </div>
      </div>
      <div style={{ padding: '36px 24px 28px', textAlign: 'center' }}>
        <div style={{
          width: 40, height: 40, margin: '0 auto 14px',
          background: 'var(--paper-2)', border: '1px solid var(--line)', borderRadius: 12,
          display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--ink-4)',
        }}>
          <Ico name="search" size={18} />
        </div>
        <div className="t-h3" style={{ marginBottom: 6 }}>No matches for “nota-acuerdo.pdf”</div>
        <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 14, maxWidth: 400, margin: '0 auto 14px', lineHeight: 1.5 }}>
          We search filenames on your device — file contents stay encrypted on our servers. If it was in a shared folder, ask the owner to re-share.
        </div>
        <div style={{ display: 'flex', gap: 6, justifyContent: 'center', flexWrap: 'wrap' }}>
          <BBBtn size="sm" icon={<Ico name="clock" size={11} />}>Check Trash</BBBtn>
          <BBBtn size="sm" icon={<Ico name="users" size={11} />}>Search shared with me</BBBtn>
          <BBBtn size="sm" variant="ghost">Clear filters</BBBtn>
        </div>
      </div>
    </div>
  );
}

// ── 3. Upload failed ────────────────────────────────────────────
function HiUploadError() {
  return (
    <div className="bb-card elevated" style={{ width: 520 }}>
      <div style={{ padding: '14px 20px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center' }}>
        <div className="t-h3">Upload</div>
        <BBChip style={{ marginLeft: 'auto', borderColor: 'oklch(0.85 0.08 25)', color: 'var(--red)' }}>
          <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--red)' }} /> 1 failed · 2 paused
        </BBChip>
      </div>

      <div style={{ padding: '14px 20px 18px' }}>
        {/* Banner */}
        <div style={{
          padding: '12px 14px', marginBottom: 14,
          background: 'oklch(0.98 0.02 25)', border: '1px solid oklch(0.88 0.05 25)',
          borderRadius: 'var(--r-2)', display: 'flex', alignItems: 'flex-start', gap: 10,
        }}>
          <div style={{
            width: 22, height: 22, borderRadius: 999, flexShrink: 0,
            background: 'oklch(0.93 0.06 25)', color: 'var(--red)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Ico name="more" size={11} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'oklch(0.4 0.15 25)' }}>
              Connection to Frankfurt dropped
            </div>
            <div style={{ fontSize: 12, color: 'var(--ink-2)', marginTop: 2, lineHeight: 1.5 }}>
              Chunks already encrypted stay safe. We can pick up where we left off — no re-encryption needed.
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {[
            { n: 'quarterly-report.pdf', size: '4.2 MB', p: 100, stage: 'Done', tone: 'ok' },
            { n: 'team-photo.heic', size: '8.1 MB', p: 78, stage: 'Paused', tone: 'warn' },
            { n: 'design-spec.fig', size: '12 MB', p: 42, stage: 'Failed · ECONNRESET', tone: 'err' },
          ].map((f, i) => {
            const c = f.tone === 'ok' ? 'oklch(0.45 0.12 155)'
                  : f.tone === 'warn' ? 'var(--ink-3)'
                  : 'var(--red)';
            return (
              <div key={i}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 5 }}>
                  <Ico name="file" size={12} color="var(--ink-3)" />
                  <span style={{ fontSize: 12.5, fontWeight: 500 }}>{f.n}</span>
                  <span className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-4)' }}>{f.size}</span>
                  <span className="t-mono" style={{ marginLeft: 'auto', fontSize: 10.5, color: c, fontWeight: 500 }}>{f.stage}</span>
                </div>
                <div className="bb-progress">
                  <div style={{ width: `${f.p}%`, background: f.tone === 'err' ? 'oklch(0.85 0.1 25)' : f.tone === 'ok' ? 'var(--ink)' : 'var(--amber)' }} />
                </div>
              </div>
            );
          })}
        </div>

        <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
          <BBBtn variant="amber" icon={<Ico name="arrowUp" size={12} />}>Resume all</BBBtn>
          <BBBtn size="sm" variant="ghost">View error details</BBBtn>
          <BBBtn size="sm" variant="ghost" style={{ marginLeft: 'auto' }}>Cancel</BBBtn>
        </div>
      </div>
    </div>
  );
}

// ── 4. Shared link expired (recipient view) ─────────────────────
function HiLinkExpired() {
  return (
    <div style={{
      width: 560, minHeight: 480, padding: 36,
      background: 'var(--paper-2)', borderRadius: 'var(--r-3)',
      border: '1px solid var(--line)', display: 'flex', flexDirection: 'column',
    }}>
      <BBLogo size={14} />
      <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '28px 0' }}>
        <div style={{ maxWidth: 420, textAlign: 'center' }}>
          <div style={{
            width: 52, height: 52, margin: '0 auto 16px',
            background: 'var(--paper)', border: '1px solid var(--line)',
            borderRadius: 16, display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'var(--ink-3)',
          }}>
            <Ico name="clock" size={22} />
          </div>
          <div className="t-h2" style={{ marginBottom: 10 }}>This link has expired</div>
          <div style={{ fontSize: 13, color: 'var(--ink-3)', marginBottom: 20, lineHeight: 1.55 }}>
            Links from Beebeeb self-destruct by design. The sender set this one to expire after <span className="t-mono" style={{ color: 'var(--ink-2)' }}>24 hours</span> or <span className="t-mono" style={{ color: 'var(--ink-2)' }}>3 opens</span>, whichever came first.
          </div>

          <div style={{
            padding: '12px 14px', marginBottom: 16,
            background: 'var(--paper)', border: '1px solid var(--line)', borderRadius: 'var(--r-2)',
            textAlign: 'left', display: 'flex', alignItems: 'center', gap: 12,
          }}>
            <div style={{
              width: 32, height: 32, borderRadius: 999, background: 'var(--amber)',
              color: 'var(--ink)', fontWeight: 700, fontSize: 12,
              display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
            }}>AK</div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12.5, fontWeight: 500 }}>Anna Kovač</div>
              <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>shared <span style={{ color: 'var(--ink-2)' }}>term-sheet-v3.docx</span> · expired 2h ago</div>
            </div>
          </div>

          <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }}>
            <Ico name="share" size={13} /> Request a new link
          </BBBtn>
          <div style={{ fontSize: 11, color: 'var(--ink-4)', marginTop: 12 }}>
            Beebeeb never saw the file's contents. We can't re-open expired links for you.
          </div>
        </div>
      </div>
    </div>
  );
}

// ── 5. Forgot recovery phrase — the hardest state ───────────────
function HiForgotPhrase() {
  return (
    <div className="bb-card elevated" style={{ width: 620, overflow: 'hidden' }}>
      <div style={{ padding: '14px 22px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center' }}>
        <BBLogo size={14} />
        <BBChip style={{ marginLeft: 'auto', borderColor: 'oklch(0.85 0.08 25)', color: 'var(--red)' }}>
          Account recovery
        </BBChip>
      </div>

      {/* Hero — honest */}
      <div style={{ padding: '28px 28px 20px', background: 'var(--paper-2)' }}>
        <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start' }}>
          <div style={{
            width: 44, height: 44, flexShrink: 0,
            background: 'oklch(0.98 0.02 25)', border: '1px solid oklch(0.88 0.05 25)',
            borderRadius: 12, display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'var(--red)',
          }}>
            <Ico name="key" size={20} />
          </div>
          <div style={{ flex: 1 }}>
            <div className="t-h1" style={{ fontSize: 24, marginBottom: 8 }}>We can't recover this for you.</div>
            <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.55 }}>
              Your recovery phrase is the only key to the vault. We never stored a copy. Not a backup, not a reset link, not a support override. That's the tradeoff you chose when you signed up for zero-knowledge encryption — and it's what keeps everyone else out.
            </div>
          </div>
        </div>
      </div>

      {/* Still-possible paths */}
      <div style={{ padding: '20px 28px' }}>
        <div className="t-label" style={{ marginBottom: 12 }}>What you can still try</div>

        {[
          {
            ico: 'users',
            ok: true,
            t: 'Ask your trusted contact',
            d: 'You set up 1 trusted helper: Marc Dupont. They can authorize recovery with their own key. Takes a 72h waiting period.',
            cta: 'Send request to Marc',
          },
          {
            ico: 'cloud',
            ok: true,
            t: 'Check other devices',
            d: 'Your iPhone 15 is still signed in. Open Beebeeb there and re-export the phrase from Settings → Security.',
            cta: 'Show me how',
          },
          {
            ico: 'download',
            ok: true,
            t: 'Find the PDF you downloaded',
            d: 'We nudged you to save one on 14 Apr 2026 during onboarding. Search your Downloads folder for "beebeeb-recovery.pdf".',
            cta: null,
          },
        ].map((p, i) => (
          <div key={i} style={{
            display: 'flex', gap: 12, padding: '12px 0',
            borderBottom: '1px solid var(--line)',
          }}>
            <div style={{
              width: 28, height: 28, flexShrink: 0, borderRadius: 8,
              background: 'oklch(0.94 0.06 155)', color: 'oklch(0.45 0.12 155)',
              border: '1px solid oklch(0.85 0.09 155)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Ico name={p.ico} size={13} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 2 }}>{p.t}</div>
              <div style={{ fontSize: 12, color: 'var(--ink-3)', lineHeight: 1.5 }}>{p.d}</div>
            </div>
            {p.cta && <BBBtn size="sm" style={{ alignSelf: 'flex-start' }}>{p.cta}</BBBtn>}
          </div>
        ))}

        {/* Dead end option — kept honest */}
        <div style={{
          marginTop: 18, padding: 14,
          background: 'oklch(0.99 0.008 25)', border: '1px solid oklch(0.9 0.04 25)',
          borderRadius: 'var(--r-2)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <Ico name="trash" size={13} color="var(--red)" />
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--red)' }}>Start over</div>
          </div>
          <div style={{ fontSize: 12, color: 'var(--ink-2)', marginBottom: 10, lineHeight: 1.5 }}>
            If none of the above works, your files are lost — mathematically, not administratively. We can wipe the encrypted blobs from our servers and give you a new account with the same email.
          </div>
          <BBBtn size="sm" variant="danger">Shred data & create new account</BBBtn>
        </div>
      </div>
    </div>
  );
}

// ── 6. Offline / sync paused (desktop) ──────────────────────────
function HiOfflineState() {
  return (
    <div className="bb-card elevated" style={{ width: 340, fontFamily: 'var(--font-sans)', borderRadius: 12 }}>
      <div style={{ padding: '12px 14px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{
          width: 26, height: 26, borderRadius: 999,
          background: 'var(--paper-2)', border: '1px solid var(--line)',
          display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--ink-3)',
        }}>
          <Ico name="cloud" size={13} />
        </div>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Offline — working locally</div>
          <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>3 changes queued · 18 MB</div>
        </div>
        <BBBtn size="sm" variant="ghost" icon={<Ico name="settings" size={13} />} />
      </div>
      <div style={{ padding: '14px 14px 10px', fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.5 }}>
        Your edits are encrypted and saved on disk. They'll sync to Beebeeb automatically when you reconnect.
      </div>
      <div style={{ padding: '0 14px 12px' }}>
        {['notes.md (edited)', 'screenshot-2026-04-22.png (new)', 'clients/ (renamed)'].map((n, i) => (
          <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 0', fontSize: 12 }}>
            <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--amber)' }} />
            <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{n}</span>
            <span className="t-mono" style={{ fontSize: 10, color: 'var(--ink-4)' }}>queued</span>
          </div>
        ))}
      </div>
      <div style={{ padding: '9px 14px', borderTop: '1px solid var(--line)', background: 'var(--paper-2)', display: 'flex', alignItems: 'center', fontSize: 11, color: 'var(--ink-3)' }}>
        <Ico name="lock" size={11} color="var(--amber-deep)" />
        <span style={{ marginLeft: 6 }}>Local changes encrypted</span>
        <span style={{ marginLeft: 'auto' }}><BBBtn size="sm" variant="ghost">Retry</BBBtn></span>
      </div>
    </div>
  );
}

Object.assign(window, { HiEmptyDrive, HiEmptySearch, HiUploadError, HiLinkExpired, HiForgotPhrase, HiOfflineState });
