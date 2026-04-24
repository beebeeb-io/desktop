// hifi-preview.jsx — Wave 2: file preview (image / PDF / video / doc)

function PreviewChrome({ filename, kind, size, children, actions = true, rightRail }) {
  const { tweaks } = (window.useTweaks && window.useTweaks()) || { tweaks: { previewTheme: 'light' } };
  const dark = tweaks.previewTheme === 'dark';
  const t = dark ? {
    bg: 'oklch(0.12 0.005 70)', panel: 'oklch(0.14 0.005 70)', rail: 'oklch(0.16 0.005 70)',
    btn: 'oklch(0.2 0.005 70)', line: 'oklch(0.22 0.005 70)',
    text: 'var(--paper)', sub: 'oklch(0.85 0.01 80)', dim: 'oklch(0.6 0.01 80)',
    canvas: 'oklch(0.08 0.005 70)',
  } : {
    bg: 'var(--paper)', panel: 'var(--paper)', rail: 'var(--paper-2)',
    btn: 'var(--paper-2)', line: 'var(--line)',
    text: 'var(--ink)', sub: 'var(--ink-2)', dim: 'var(--ink-3)',
    canvas: 'var(--paper-2)',
  };
  window.__previewTokens = t; // InfoRail reads this
  return (
    <div style={{
      width: '100%', height: '100%', background: t.bg,
      display: 'flex', flexDirection: 'column', color: t.text,
      borderRadius: 'var(--r-3)', overflow: 'hidden',
      border: dark ? 'none' : '1px solid var(--line)',
    }}>
      {/* Top bar */}
      <div style={{
        height: 48, display: 'flex', alignItems: 'center', padding: '0 14px', gap: 12,
        borderBottom: `1px solid ${t.line}`, background: t.panel,
      }}>
        <div style={{
          width: 28, height: 28, borderRadius: 6, background: t.btn,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <Ico name="chevRight" size={13} color={t.text} />
        </div>
        <div style={{ flex: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{
            width: 22, height: 22, borderRadius: 5, background: t.btn,
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Ico name={kind === 'image' ? 'image' : 'file'} size={12} color={t.text} />
          </span>
          <span style={{ fontSize: 13, fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: t.text }}>
            {filename}
          </span>
          <span className="t-mono" style={{ fontSize: 11, color: t.dim }}>{size}</span>
          <BBChip variant="amber" style={{ fontSize: 9, marginLeft: 6 }}>
            <Ico name="lock" size={9} /> Decrypted locally
          </BBChip>
        </div>
        {actions && (
          <div style={{ display: 'flex', gap: 6 }}>
            {['download', 'share', 'star', 'more'].map(n => (
              <div key={n} style={{
                width: 28, height: 28, borderRadius: 6, background: t.btn,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                color: t.sub,
              }}>
                <Ico name={n} size={13} color={t.sub} />
              </div>
            ))}
          </div>
        )}
      </div>
      {/* Body */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0, background: t.canvas }}>
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 24, position: 'relative' }}>
          {children}
        </div>
        {rightRail && (
          <div style={{
            width: 280, background: t.rail, borderLeft: `1px solid ${t.line}`,
            padding: 18, overflow: 'auto', color: t.sub,
          }}>{rightRail}</div>
        )}
      </div>
    </div>
  );
}

function InfoRail({ filename, kind, size, items }) {
  const t = window.__previewTokens || { dim: 'var(--ink-3)', line: 'var(--line)', sub: 'var(--ink-2)', text: 'var(--ink)' };
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <div>
        <div className="t-label" style={{ color: t.dim, marginBottom: 8 }}>Details</div>
        {[
          ['Name', filename],
          ['Size', size],
          ['Kind', kind],
          ...items,
        ].map(([k, v], i) => (
          <div key={i} style={{ display: 'flex', padding: '5px 0', fontSize: 12, borderBottom: `1px solid ${t.line}` }}>
            <span style={{ width: 90, color: t.dim }}>{k}</span>
            <span className="t-mono" style={{ flex: 1, fontSize: 11, wordBreak: 'break-all', color: t.text }}>{v}</span>
          </div>
        ))}
      </div>
      <div>
        <div className="t-label" style={{ color: t.dim, marginBottom: 8 }}>Crypto</div>
        <div style={{ fontSize: 11, fontFamily: 'var(--font-mono)', color: t.sub, lineHeight: 1.8 }}>
          XChaCha20-Poly1305<br />
          256-bit vault key<br />
          IV: 8f2e…91a3<br />
          MAC verified ✓
        </div>
      </div>
    </div>
  );
}

// ─── Image preview ─────────────────────────────────────
function HiPreviewImage() {
  // Fake photo: protest scene abstract
  return (
    <PreviewChrome
      filename="protest-20250914-berlin-0147.cr3"
      kind="image"
      size="42.8 MB"
      rightRail={<InfoRail
        filename="protest-20250914-berlin-0147.cr3"
        kind="Canon Raw 3"
        size="42.8 MB"
        items={[
          ['Dimensions', '6240 × 4160'],
          ['Taken', '14 Sep 2025 · 18:42'],
          ['Camera', 'Canon R6 Mark II'],
          ['Location', 'stripped'],
          ['Modified', '2 hours ago'],
        ]}
      />}
    >
      {/* Abstract photo */}
      <div style={{
        width: '100%', height: '100%', maxWidth: 560, aspectRatio: '3/2',
        borderRadius: 6, overflow: 'hidden', position: 'relative',
        background: 'linear-gradient(160deg, oklch(0.45 0.08 40) 0%, oklch(0.28 0.06 35) 40%, oklch(0.18 0.04 30) 100%)',
        boxShadow: '0 20px 60px -10px rgba(0,0,0,0.6)',
      }}>
        {/* Figures */}
        <svg viewBox="0 0 600 400" width="100%" height="100%" style={{ position: 'absolute', inset: 0 }}>
          <defs>
            <radialGradient id="flare" cx="50%" cy="30%">
              <stop offset="0%" stopColor="oklch(0.9 0.15 75)" stopOpacity="0.6" />
              <stop offset="100%" stopColor="oklch(0.9 0.15 75)" stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect x="0" y="0" width="600" height="400" fill="url(#flare)" />
          {/* Crowd silhouettes */}
          {Array.from({ length: 28 }).map((_, i) => {
            const x = 30 + (i % 14) * 42 + ((i * 17) % 20);
            const y = 240 + Math.floor(i / 14) * 40 + ((i * 11) % 15);
            const h = 80 + ((i * 7) % 40);
            return (
              <g key={i} fill="oklch(0.1 0.02 30)" opacity={0.88}>
                <circle cx={x} cy={y - h + 10} r="12" />
                <rect x={x - 14} y={y - h + 22} width="28" height={h - 22} rx="6" />
              </g>
            );
          })}
          {/* Smoke */}
          <ellipse cx="320" cy="140" rx="180" ry="80" fill="oklch(0.7 0.04 60)" opacity="0.3" />
          <ellipse cx="180" cy="100" rx="120" ry="50" fill="oklch(0.6 0.02 60)" opacity="0.4" />
        </svg>
        {/* EXIF stripped badge */}
        <div style={{
          position: 'absolute', bottom: 12, left: 12,
          padding: '5px 10px', borderRadius: 999, fontSize: 10,
          background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(8px)',
          color: 'var(--amber)', fontFamily: 'var(--font-mono)',
          border: '1px solid oklch(0.82 0.17 84 / 0.3)',
        }}>
          ⚠ GPS & device serial stripped before encryption
        </div>
      </div>
    </PreviewChrome>
  );
}

// ─── PDF preview ─────────────────────────────────────
function HiPreviewPDF() {
  return (
    <PreviewChrome
      filename="source-interview-transcript-redacted.pdf"
      kind="application/pdf"
      size="384 KB"
      rightRail={<InfoRail
        filename="source-interview-transcript-redacted.pdf"
        kind="PDF · 8 pages"
        size="384 KB"
        items={[
          ['Created', '12 Sep 2025'],
          ['Modified', 'yesterday'],
          ['Pages', '8'],
          ['Shared', '1 link · 48h expiry'],
        ]}
      />}
    >
      <div style={{
        width: 520, background: 'var(--paper)', borderRadius: 4,
        boxShadow: '0 10px 40px rgba(0,0,0,0.4)', padding: '42px 52px 32px',
        color: 'var(--ink)', fontSize: 11.5, lineHeight: 1.65,
      }}>
        <div style={{ fontSize: 18, fontWeight: 700, letterSpacing: '-0.015em', marginBottom: 6 }}>
          Interview transcript · Source B
        </div>
        <div className="t-mono" style={{ fontSize: 10, color: 'var(--ink-3)', marginBottom: 22 }}>
          12 Sep 2025 · encrypted location · 43 min
        </div>

        <div style={{ marginBottom: 14 }}>
          <strong>Q.</strong> Let's start with how you first noticed the irregularity.
        </div>
        <div style={{ marginBottom: 14 }}>
          <strong>B.</strong> It was in ████████. Around the time they were preparing the ████ report. I pulled the logs and there was a gap — ██ hours unaccounted for.
        </div>
        <div style={{ marginBottom: 14 }}>
          <strong>Q.</strong> And you reported this internally?
        </div>
        <div style={{ marginBottom: 14 }}>
          <strong>B.</strong> I did. To ████████ and then to ████████. Neither responded in writing. That was the first signal that something was wrong.
        </div>
        <div style={{ marginBottom: 14 }}>
          <strong>Q.</strong> Did anyone else on the team know?
        </div>
        <div style={{ marginBottom: 14, color: 'var(--ink-3)' }}>
          [████████ 14 lines redacted per agreement ████████]
        </div>
        <div style={{ marginBottom: 14 }}>
          <strong>B.</strong> After that, I started keeping copies off the network. I bought a cheap drive and a second one for backup. I didn't trust email anymore.
        </div>
        <div style={{ marginTop: 32, paddingTop: 12, borderTop: '1px solid var(--line)', fontSize: 9.5, color: 'var(--ink-4)', display: 'flex' }}>
          <span>page 1 of 8</span>
          <span style={{ marginLeft: 'auto' }}>IM · redacted</span>
        </div>
      </div>
      {/* Page nav */}
      <div style={{
        position: 'absolute', bottom: 20, left: '50%', transform: 'translateX(-50%)',
        display: 'flex', alignItems: 'center', gap: 0,
        background: 'oklch(0.2 0.005 70 / 0.9)', borderRadius: 999, padding: 4,
        backdropFilter: 'blur(8px)', border: '1px solid oklch(0.25 0.005 70)',
      }}>
        <div style={{ padding: '6px 10px', color: 'oklch(0.85 0.01 80)', transform: 'rotate(180deg)' }}><Ico name="chevRight" size={13} color="currentColor" /></div>
        <span className="t-mono" style={{ fontSize: 11, padding: '0 8px', color: 'var(--paper)' }}>1 / 8</span>
        <div style={{ padding: '6px 10px', color: 'oklch(0.85 0.01 80)' }}><Ico name="chevRight" size={13} /></div>
      </div>
    </PreviewChrome>
  );
}

// ─── Video preview ─────────────────────────────────────
function HiPreviewVideo() {
  return (
    <PreviewChrome
      filename="field-recording-20250914.mp4"
      kind="video/mp4"
      size="1.2 GB"
      rightRail={<InfoRail
        filename="field-recording-20250914.mp4"
        kind="H.264 · 4K · 30fps"
        size="1.2 GB"
        items={[
          ['Duration', '12:47'],
          ['Recorded', '14 Sep 2025'],
          ['Bitrate', '14.2 Mbps'],
          ['Audio', 'AAC · stereo'],
        ]}
      />}
    >
      <div style={{
        width: '100%', maxWidth: 640, aspectRatio: '16/9', borderRadius: 6,
        background: 'oklch(0.08 0.01 50)', position: 'relative', overflow: 'hidden',
        boxShadow: '0 20px 60px -10px rgba(0,0,0,0.6)',
      }}>
        <svg viewBox="0 0 640 360" width="100%" height="100%" style={{ position: 'absolute', inset: 0 }}>
          {/* Street scene — rough */}
          <rect width="640" height="200" fill="oklch(0.25 0.04 40)" />
          <rect y="200" width="640" height="160" fill="oklch(0.18 0.02 40)" />
          {/* Buildings */}
          <rect x="40" y="80" width="120" height="120" fill="oklch(0.15 0.01 40)" />
          <rect x="180" y="60" width="100" height="140" fill="oklch(0.18 0.01 40)" />
          <rect x="300" y="100" width="140" height="100" fill="oklch(0.12 0.01 40)" />
          <rect x="460" y="70" width="140" height="130" fill="oklch(0.16 0.01 40)" />
          {/* Windows */}
          {Array.from({ length: 40 }).map((_, i) => (
            <rect key={i} x={50 + (i % 10) * 55 + ((i * 7) % 12)} y={85 + Math.floor(i / 10) * 25}
              width="6" height="10" fill={i % 3 === 0 ? 'oklch(0.7 0.12 75)' : 'oklch(0.3 0.02 50)'} />
          ))}
          {/* Moving figure */}
          <circle cx="320" cy="250" r="10" fill="oklch(0.15 0.01 30)" />
          <rect x="312" y="258" width="16" height="50" fill="oklch(0.15 0.01 30)" />
        </svg>
        {/* Play button */}
        <div style={{
          position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%, -50%)',
          width: 64, height: 64, borderRadius: '50%', background: 'rgba(0,0,0,0.5)',
          backdropFilter: 'blur(8px)', border: '1.5px solid rgba(255,255,255,0.8)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{
            width: 0, height: 0, borderTop: '12px solid transparent', borderBottom: '12px solid transparent',
            borderLeft: '18px solid var(--paper)', marginLeft: 4,
          }} />
        </div>
        {/* Controls */}
        <div style={{
          position: 'absolute', bottom: 0, left: 0, right: 0, padding: 14,
          background: 'linear-gradient(0deg, rgba(0,0,0,0.7), transparent)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginBottom: 8 }}>
            <div style={{ height: 3, flex: 1, background: 'rgba(255,255,255,0.2)', borderRadius: 2, position: 'relative' }}>
              <div style={{ position: 'absolute', left: 0, top: 0, bottom: 0, width: '34%', background: 'var(--amber)', borderRadius: 2 }} />
              <div style={{ position: 'absolute', left: '34%', top: -4, width: 11, height: 11, borderRadius: '50%', background: 'var(--amber)', transform: 'translateX(-50%)' }} />
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: 'var(--paper)' }}>
            <span className="t-mono" style={{ fontSize: 11 }}>04:22 / 12:47</span>
          </div>
        </div>
      </div>
    </PreviewChrome>
  );
}

// ─── Doc preview (markdown-ish note) ─────────────────────────────────────
function HiPreviewDoc() {
  return (
    <PreviewChrome
      filename="story-draft.md"
      kind="text/markdown"
      size="18 KB"
      rightRail={<InfoRail
        filename="story-draft.md"
        kind="Markdown"
        size="18 KB"
        items={[
          ['Words', '4,820'],
          ['Lines', '214'],
          ['Modified', '4 min ago'],
          ['Versions', '47'],
        ]}
      />}
    >
      <div style={{
        width: 620, maxHeight: '92%', overflow: 'auto',
        background: 'var(--paper)', borderRadius: 6,
        boxShadow: '0 10px 40px rgba(0,0,0,0.4)', padding: '42px 56px 48px',
        color: 'var(--ink)',
      }}>
        <div style={{ fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', marginBottom: 8, lineHeight: 1.15 }}>
          The gap in the ledger
        </div>
        <div className="t-mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', marginBottom: 24, letterSpacing: '0.04em', textTransform: 'uppercase' }}>
          Draft · Berlin · 22 Apr 2026
        </div>
        <div style={{ fontSize: 13, lineHeight: 1.7, color: 'var(--ink-2)' }}>
          <p style={{ margin: '0 0 12px' }}>
            For eleven hours on a Tuesday in September, no one at the agency logged a single transaction. The system was not down. The auditors were not on holiday. According to seven people familiar with the matter, the silence was deliberate.
          </p>
          <p style={{ margin: '0 0 12px' }}>
            What happened in those hours — and who decided that it should not be recorded — is the subject of this investigation.
          </p>
          <div style={{ fontSize: 16, fontWeight: 600, margin: '28px 0 8px' }}>I. The call</div>
          <p style={{ margin: '0 0 12px' }}>
            Source B was a mid-level compliance officer. They remember the call because it was unusual in two respects: it came from a number outside the organisation, and the person on the line asked them to do something they had never been asked to do in fourteen years on the job.
          </p>
          <p style={{ margin: '0 0 12px', color: 'var(--ink-4)' }}>
            [continues — 3,800 words]
          </p>
        </div>
      </div>
    </PreviewChrome>
  );
}

Object.assign(window, { HiPreviewImage, HiPreviewPDF, HiPreviewVideo, HiPreviewDoc });
