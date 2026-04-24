// hifi-web-extra.jsx — Tier 1: recipient share, password prompt, file details, context menu

// ─── Browser-ish frame for public / recipient pages ─────────────────────────
function WebFrame({ url = 'beebeeb.io', children, width = 1080, height = 680 }) {
  return (
    <div style={{
      width, background: 'var(--paper)',
      borderRadius: 12, overflow: 'hidden',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.25), 0 0 0 1px var(--line-2)',
    }}>
      <div style={{
        height: 36, background: 'var(--paper-2)', borderBottom: '1px solid var(--line)',
        display: 'flex', alignItems: 'center', padding: '0 14px', gap: 10,
      }}>
        <div style={{ display: 'flex', gap: 6 }}>
          {['#ff5f57', '#febc2e', '#28c840'].map((c, i) => (
            <div key={i} style={{ width: 11, height: 11, borderRadius: '50%', background: c }} />
          ))}
        </div>
        <div style={{
          flex: 1, background: 'var(--paper)', borderRadius: 6,
          padding: '4px 12px', fontSize: 11.5, color: 'var(--ink-2)',
          display: 'flex', alignItems: 'center', gap: 7,
          border: '1px solid var(--line)', maxWidth: 560, margin: '0 auto',
        }}>
          <Ico name="lock" size={10} color="oklch(0.55 0.14 155)" />
          <span className="t-mono" style={{ fontSize: 11 }}>{url}</span>
        </div>
      </div>
      <div style={{ height: height - 36, overflow: 'hidden' }}>{children}</div>
    </div>
  );
}

// ─── 1. Recipient share view — someone opens a beebeeb.io/s/... link ───────
function HiRecipientShare() {
  return (
    <WebFrame url="beebeeb.io/s/kF2x·9qP4#m3Nq...vR8" height={680}>
      <div style={{ height: '100%', background: 'var(--paper-2)', display: 'flex', flexDirection: 'column' }}>
        <div style={{
          padding: '10px 22px', display: 'flex', alignItems: 'center', gap: 12,
          borderBottom: '1px solid var(--line)', background: 'var(--paper)',
        }}>
          <BBLogo size={14} />
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Shared link · end-to-end encrypted</div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8, alignItems: 'center' }}>
            <BBRegionBadge region="Frankfurt" />
            <BBBtn size="sm" variant="ghost">Sign in</BBBtn>
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 320px', flex: 1, overflow: 'hidden' }}>
          <div style={{ padding: 28, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16,
              padding: '10px 14px', background: 'var(--paper)',
              borderRadius: 10, border: '1px solid var(--line)',
            }}>
              <div style={{
                width: 32, height: 32, borderRadius: '50%',
                background: 'oklch(0.7 0.1 55)', color: 'var(--paper)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 11, fontWeight: 600,
              }}>IM</div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 12.5, fontWeight: 500 }}>Isa Marchetti shared a file</div>
                <div className="t-micro" style={{ color: 'var(--ink-3)' }}>isa@example.eu · 2 hours ago · expires in 5 days</div>
              </div>
              <BBChip variant="green"><span className="dot" /> Verified sender</BBChip>
            </div>

            {/* PDF thumb */}
            <div style={{
              flex: 1, background: 'oklch(0.97 0.004 80)', border: '1px solid var(--line)',
              borderRadius: 10, display: 'flex', alignItems: 'center', justifyContent: 'center',
              position: 'relative', overflow: 'hidden',
            }}>
              <div style={{
                width: 380, height: 460, background: 'var(--paper)',
                boxShadow: '0 18px 48px -12px rgba(0,0,0,0.18)', borderRadius: 4,
                padding: '48px 40px', fontFamily: 'var(--font-serif, serif)',
              }}>
                <div style={{ fontSize: 11, letterSpacing: 1.6, color: 'var(--ink-3)', textTransform: 'uppercase' }}>Story draft · 03</div>
                <div style={{ fontSize: 22, fontWeight: 700, marginTop: 14, lineHeight: 1.15, letterSpacing: -0.3 }}>The paper trail behind the port contract</div>
                <div style={{ marginTop: 18 }}>
                  {[100, 94, 97, 88, 92, 84, 96, 79, 60, 0, 90, 86].map((w, i) => (
                    <div key={i} style={{
                      height: 5, background: w === 0 ? 'transparent' : 'oklch(0.85 0.005 80)',
                      borderRadius: 2, marginBottom: 7, width: `${w}%`,
                    }} />
                  ))}
                </div>
              </div>
              <div style={{
                position: 'absolute', bottom: 14, left: 14, right: 14,
                background: 'rgba(0,0,0,0.75)', color: 'var(--paper)',
                padding: '8px 12px', borderRadius: 6,
                display: 'flex', alignItems: 'center', gap: 10, fontSize: 11.5,
              }}>
                <Ico name="eye" size={12} />
                <span>Preview · decrypted locally in your browser</span>
                <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 10, opacity: 0.7 }}>page 1 of 18</span>
              </div>
            </div>
          </div>

          {/* right rail */}
          <div style={{
            borderLeft: '1px solid var(--line)', padding: 22,
            background: 'var(--paper)', overflow: 'auto',
            display: 'flex', flexDirection: 'column', gap: 18,
          }}>
            <div>
              <div style={{ fontSize: 15, fontWeight: 600, lineHeight: 1.2 }}>port-contract-draft-03.pdf</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 4 }}>PDF · 2.4 MB · 18 pages</div>
            </div>

            <div style={{ display: 'flex', gap: 8, flexDirection: 'column' }}>
              <BBBtn variant="amber" size="sm" style={{ width: '100%', justifyContent: 'center' }}>
                <Ico name="download" size={12} color="var(--ink)" /> Download
              </BBBtn>
              <BBBtn variant="ghost" size="sm" style={{ width: '100%', justifyContent: 'center' }}>
                Save to my vault
              </BBBtn>
            </div>

            <div style={{ padding: 12, background: 'var(--paper-2)', borderRadius: 8, border: '1px solid var(--line)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
                <Ico name="shield" size={11} color="var(--amber-deep)" />
                <span className="t-label" style={{ fontSize: 9.5 }}>Access</span>
              </div>
              {[
                ['View', true], ['Download', true], ['Comment', false], ['Re-share', false],
              ].map(([l, on], i) => (
                <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 0', fontSize: 12 }}>
                  <span style={{
                    width: 12, height: 12, borderRadius: 3,
                    background: on ? 'var(--amber)' : 'transparent',
                    border: `1px solid ${on ? 'var(--amber-deep)' : 'var(--line-2)'}`,
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    fontSize: 9, color: 'var(--ink)',
                  }}>{on && '✓'}</span>
                  <span style={{ color: on ? 'var(--ink)' : 'var(--ink-3)' }}>{l}</span>
                </div>
              ))}
            </div>

            <div>
              <div className="t-label" style={{ marginBottom: 8 }}>How this works</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', lineHeight: 1.55 }}>
                The link fragment <span className="t-mono" style={{ color: 'var(--ink-2)' }}>#m3Nq...vR8</span> after
                the <span className="t-mono">#</span> is the decryption key. It never leaves your browser — Beebeeb
                servers see only ciphertext.
              </div>
            </div>

            <div style={{ marginTop: 'auto', padding: 10, background: 'oklch(0.96 0.04 155 / 0.6)', border: '1px solid oklch(0.87 0.08 155)', borderRadius: 8 }}>
              <div className="t-micro" style={{ color: 'oklch(0.38 0.12 155)', fontWeight: 500 }}>Fingerprint match ✓</div>
              <div className="t-mono" style={{ fontSize: 9.5, color: 'oklch(0.4 0.1 155)', marginTop: 3, wordBreak: 'break-all' }}>
                a3:7c:f2:91:4b:e0:8d:5a
              </div>
            </div>
          </div>
        </div>
      </div>
    </WebFrame>
  );
}

// ─── 2. Password prompt — "this link is also passphrase-protected" ─────────
function HiRecipientPassword() {
  return (
    <WebFrame url="beebeeb.io/s/kF2x·9qP4#m3Nq...vR8" height={620}>
      <div style={{
        height: '100%', background: 'var(--paper-2)',
        display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 40,
      }}>
        <div style={{
          width: 440, background: 'var(--paper)', padding: 32,
          borderRadius: 14, border: '1px solid var(--line)',
          boxShadow: '0 18px 48px -18px rgba(0,0,0,0.12)',
        }}>
          <div style={{
            width: 48, height: 48, borderRadius: 12,
            background: 'var(--amber-bg)', border: '1px solid var(--amber-deep)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            marginBottom: 18,
          }}>
            <Ico name="lock" size={20} color="var(--amber-deep)" />
          </div>
          <div className="t-h3" style={{ fontSize: 20, fontWeight: 600, letterSpacing: -0.3 }}>
            This link needs a passphrase
          </div>
          <div className="t-body" style={{ color: 'var(--ink-2)', marginTop: 6, fontSize: 13, lineHeight: 1.5 }}>
            Isa added a second secret on top of the link key. Ask them through a different channel —
            a phone call, Signal, in person.
          </div>

          <div style={{ marginTop: 20 }}>
            <div className="t-label" style={{ marginBottom: 6 }}>Passphrase</div>
            <div className="bb-input" style={{ padding: '10px 12px' }}>
              <Ico name="key" size={12} color="var(--ink-3)" />
              <input type="password" placeholder="Enter the passphrase Isa gave you" defaultValue="••••••••••••" />
              <Ico name="eye" size={12} color="var(--ink-3)" />
            </div>
          </div>

          <BBBtn variant="amber" size="md" style={{ width: '100%', justifyContent: 'center', marginTop: 14 }}>
            <Ico name="lock" size={12} color="var(--ink)" /> Unlock
          </BBBtn>

          <div style={{
            marginTop: 20, padding: 12,
            background: 'var(--paper-2)', border: '1px solid var(--line)',
            borderRadius: 8, display: 'flex', gap: 10, alignItems: 'flex-start',
          }}>
            <Ico name="shield" size={12} color="var(--ink-3)" style={{ marginTop: 2 }} />
            <div>
              <div style={{ fontSize: 11.5, fontWeight: 500 }}>We never see the passphrase</div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2, lineHeight: 1.5 }}>
                It stays in your browser, combined with the link key to decrypt the file.
                Fail three times and the link burns itself.
              </div>
            </div>
          </div>

          <div className="t-micro" style={{ color: 'var(--ink-4)', marginTop: 14, textAlign: 'center' }}>
            3 attempts remaining · Link expires in 5 days
          </div>
        </div>
      </div>
    </WebFrame>
  );
}

Object.assign(window, { WebFrame, HiRecipientShare, HiRecipientPassword });
