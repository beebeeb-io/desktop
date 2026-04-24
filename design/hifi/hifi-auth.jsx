// hifi-auth.jsx — Wave 1: auth flows
// signup, login, email verify, 2FA, passkeys, forgot pw, session mgmt, account delete

function AuthShell({ children, width = 440, title, subtitle, step, steps }) {
  return (
    <div style={{
      width, background: 'var(--paper)', borderRadius: 'var(--r-3)',
      border: '1px solid var(--line-2)', boxShadow: 'var(--shadow-3)',
      overflow: 'hidden',
    }}>
      <div style={{ padding: '22px 24px 18px', borderBottom: '1px solid var(--line)' }}>
        <BBLogo size={14} />
        {steps && (
          <div style={{ display: 'flex', gap: 4, marginTop: 14 }}>
            {Array.from({ length: steps }).map((_, i) => (
              <div key={i} style={{
                flex: 1, height: 3, borderRadius: 2,
                background: i < step ? 'var(--amber)' : 'var(--paper-3)',
              }} />
            ))}
          </div>
        )}
      </div>
      <div style={{ padding: '22px 24px 24px' }}>
        <div className="t-h2" style={{ marginBottom: 6 }}>{title}</div>
        {subtitle && <div style={{ fontSize: 13, color: 'var(--ink-3)', marginBottom: 20, lineHeight: 1.5 }}>{subtitle}</div>}
        {children}
      </div>
    </div>
  );
}

function Field({ label, children, hint, right }) {
  return (
    <div style={{ marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 6 }}>
        <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--ink-2)' }}>{label}</label>
        {right && <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--amber-deep)', cursor: 'pointer' }}>{right}</span>}
      </div>
      {children}
      {hint && <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 5, lineHeight: 1.4 }}>{hint}</div>}
    </div>
  );
}

// ─── Signup ─────────────────────────────────────
function HiSignup() {
  return (
    <AuthShell title="Create your account" subtitle="Encrypted end-to-end before it leaves your device. We can't read any of it.">
      <Field label="Email">
        <div className="bb-input">
          <input defaultValue="isa.marchetti@example.com" />
        </div>
      </Field>
      <Field label="Password" hint="Used to unlock your vault on this device. Your email provider never sees it.">
        <div className="bb-input">
          <input type="password" defaultValue="•••••••••••••••" />
          <Ico name="eye" size={13} color="var(--ink-3)" />
        </div>
        <div style={{ marginTop: 8, display: 'flex', gap: 4 }}>
          {[1, 1, 1, 1].map((s, i) => (
            <div key={i} style={{ flex: 1, height: 3, background: 'var(--green)', borderRadius: 2, opacity: 0.9 }} />
          ))}
        </div>
        <div className="t-micro" style={{ marginTop: 4, color: 'oklch(0.5 0.12 155)' }}>Strong — 18 chars</div>
      </Field>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, marginTop: 10, marginBottom: 14 }}>
        <BBCheck on />
        <div style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.5 }}>
          I understand that <strong>Beebeeb cannot recover my account</strong> if I lose both my password and recovery phrase.
        </div>
      </div>
      <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }}>Create account</BBBtn>
      <div style={{ fontSize: 12, color: 'var(--ink-3)', textAlign: 'center', marginTop: 16 }}>
        Already have an account? <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>Log in →</span>
      </div>
      <div style={{ borderTop: '1px solid var(--line)', marginTop: 18, paddingTop: 14, display: 'flex', alignItems: 'center', gap: 8, fontSize: 11, color: 'var(--ink-3)' }}>
        <Ico name="shield" size={12} color="var(--amber-deep)" />
        <span>Stored in Frankfurt · Hetzner · under EU jurisdiction</span>
      </div>
    </AuthShell>
  );
}

// ─── Login ─────────────────────────────────────
function HiLogin() {
  return (
    <AuthShell title="Welcome back" subtitle="Log in to unlock your vault.">
      <Field label="Email">
        <div className="bb-input">
          <input defaultValue="isa.marchetti@example.com" />
        </div>
      </Field>
      <Field label="Password" right="Forgot?">
        <div className="bb-input">
          <input type="password" defaultValue="•••••••••••••••" />
          <Ico name="eye" size={13} color="var(--ink-3)" />
        </div>
      </Field>
      <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center', marginTop: 6 }}>Log in</BBBtn>

      <div style={{ margin: '20px 0 14px', display: 'flex', alignItems: 'center', gap: 10, fontSize: 11, color: 'var(--ink-4)' }}>
        <div style={{ flex: 1, height: 1, background: 'var(--line)' }} />
        <span>OR</span>
        <div style={{ flex: 1, height: 1, background: 'var(--line)' }} />
      </div>
      <BBBtn style={{ width: '100%', justifyContent: 'center' }} icon={<Ico name="key" size={12} />}>
        Sign in with passkey
      </BBBtn>

      <div style={{ fontSize: 12, color: 'var(--ink-3)', textAlign: 'center', marginTop: 20 }}>
        New to Beebeeb? <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>Create an account →</span>
      </div>
    </AuthShell>
  );
}

// ─── Email verify ─────────────────────────────────────
function HiEmailVerify() {
  return (
    <AuthShell title="Check your inbox" subtitle="We sent a verification code to isa.marchetti@example.com. Enter it below.">
      <div style={{ display: 'flex', gap: 8, marginBottom: 14, justifyContent: 'center' }}>
        {['7', '3', '9', '2', '1', ''].map((d, i) => (
          <div key={i} style={{
            width: 44, height: 52, display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: 'var(--font-mono)', fontSize: 24, fontWeight: 600,
            border: '1.5px solid', borderColor: i === 5 ? 'var(--amber-deep)' : 'var(--line-2)',
            borderRadius: 'var(--r-2)',
            background: i === 5 ? 'var(--paper)' : 'var(--paper)',
            boxShadow: i === 5 ? 'var(--ring)' : 'none',
            color: d ? 'var(--ink)' : 'var(--ink-4)',
          }}>{d || '|'}</div>
        ))}
      </div>
      <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center', opacity: 0.5 }}>Verify email</BBBtn>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 16, fontSize: 11.5 }}>
        <span style={{ color: 'var(--ink-3)' }}>Didn't receive it?</span>
        <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>Resend in 42s</span>
      </div>
    </AuthShell>
  );
}

// ─── 2FA setup ─────────────────────────────────────
function HiTwoFactor() {
  // QR pattern — pseudo-random squares
  const grid = Array.from({ length: 21 }, (_, r) =>
    Array.from({ length: 21 }, (_, c) => {
      const corner = (r < 7 && c < 7) || (r < 7 && c > 13) || (r > 13 && c < 7);
      const edge = corner && (r === 0 || r === 6 || c === 0 || c === 6 || r === 14 || r === 20 || c === 14 || c === 20);
      const inner = corner && r >= 2 && r <= 4 && c >= 2 && c <= 4;
      const innerR = corner && r >= 2 && r <= 4 && c >= 16 && c <= 18;
      const innerB = corner && r >= 16 && r <= 18 && c >= 2 && c <= 4;
      if (edge || inner || innerR || innerB) return 1;
      return ((r * 7 + c * 13 + r * c) % 3 === 0) ? 1 : 0;
    })
  );
  return (
    <AuthShell title="Set up two-factor" subtitle="Scan with your authenticator app, then enter the 6-digit code." step={1} steps={3} width={460}>
      <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 18, alignItems: 'flex-start' }}>
        <div style={{
          background: 'var(--paper)', padding: 10, border: '1px solid var(--line-2)',
          borderRadius: 'var(--r-2)', display: 'grid', gridTemplateColumns: 'repeat(21, 1fr)', gap: 0,
          width: 160, height: 160,
        }}>
          {grid.flat().map((v, i) => (
            <div key={i} style={{ background: v ? 'var(--ink)' : 'transparent' }} />
          ))}
        </div>
        <div>
          <div className="t-label" style={{ marginBottom: 6 }}>Or enter manually</div>
          <div className="t-mono" style={{
            fontSize: 12, fontWeight: 500, padding: '8px 10px',
            background: 'var(--paper-2)', border: '1px solid var(--line)',
            borderRadius: 'var(--r-2)', marginBottom: 12, wordBreak: 'break-all', lineHeight: 1.5,
          }}>JBSW Y3DP EHPK 3PXP</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', lineHeight: 1.5 }}>
            Recommended: <strong>Aegis</strong> (Android), <strong>Raivo</strong> (iOS), <strong>1Password</strong>.
          </div>
        </div>
      </div>
      <div style={{ borderTop: '1px solid var(--line)', margin: '20px 0 16px' }} />
      <Field label="Enter 6-digit code">
        <div className="bb-input" style={{ fontFamily: 'var(--font-mono)', fontSize: 15, letterSpacing: '0.15em' }}>
          <input defaultValue="482 913" style={{ fontFamily: 'inherit', letterSpacing: 'inherit' }} />
        </div>
      </Field>
      <div style={{ display: 'flex', gap: 10, marginTop: 8 }}>
        <BBBtn style={{ flex: 1, justifyContent: 'center' }}>Skip for now</BBBtn>
        <BBBtn variant="amber" style={{ flex: 1, justifyContent: 'center' }}>Enable 2FA</BBBtn>
      </div>
    </AuthShell>
  );
}

// ─── Passkeys ─────────────────────────────────────
function HiPasskey() {
  return (
    <AuthShell title="Add a passkey" subtitle="Faster sign-in. Tied to this device's secure enclave. Replaces your password on trusted devices." width={440}>
      <div style={{
        padding: 18, background: 'var(--paper-2)', borderRadius: 'var(--r-2)',
        border: '1px solid var(--line)', marginBottom: 14,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 44, height: 44, borderRadius: 10, background: 'var(--amber-bg)',
            border: '1px solid oklch(0.86 0.07 90)', display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Ico name="key" size={20} color="var(--amber-deep)" />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 14, fontWeight: 600 }}>MacBook Pro · Touch ID</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>Safari · macOS 14.4 · detected</div>
          </div>
        </div>
      </div>

      <div style={{
        padding: 14, background: 'var(--paper)', border: '1px dashed var(--line-2)',
        borderRadius: 'var(--r-2)', display: 'flex', gap: 12, marginBottom: 18,
      }}>
        <Ico name="shield" size={14} color="var(--amber-deep)" />
        <div style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.55 }}>
          Your passkey never leaves this device. Beebeeb only sees its public half — not enough to impersonate you.
        </div>
      </div>

      <BBBtn variant="amber" size="lg" style={{ width: '100%', justifyContent: 'center' }} icon={<Ico name="key" size={13} />}>
        Create passkey with Touch ID
      </BBBtn>
      <div style={{ textAlign: 'center', marginTop: 14, fontSize: 11.5, color: 'var(--ink-3)', cursor: 'pointer' }}>
        Not now →
      </div>
    </AuthShell>
  );
}

// ─── Forgot password — the dead end done right ─────────────────────────────────────
function HiForgotPassword() {
  return (
    <AuthShell title="Reset your password" width={480}>
      <div style={{
        padding: 14, background: 'var(--amber-bg)', border: '1px solid oklch(0.86 0.07 90)',
        borderRadius: 'var(--r-2)', marginBottom: 18, display: 'flex', gap: 10,
      }}>
        <Ico name="shield" size={14} color="var(--amber-deep)" />
        <div style={{ fontSize: 12.5, color: 'oklch(0.35 0.1 72)', lineHeight: 1.55 }}>
          <strong>We don't reset passwords.</strong> Because your files are encrypted with a key derived from it, resetting would require us to read your vault — and we designed the system so we can't.
        </div>
      </div>

      <div style={{ fontSize: 13.5, color: 'var(--ink-2)', lineHeight: 1.6, marginBottom: 16 }}>
        You have two options:
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginBottom: 18 }}>
        <div style={{ padding: 14, border: '1px solid var(--line-2)', borderRadius: 'var(--r-2)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
            <span className="t-mono" style={{
              width: 22, height: 22, borderRadius: 6, background: 'var(--ink)',
              color: 'var(--paper)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              fontSize: 11, fontWeight: 600,
            }}>1</span>
            <div style={{ fontSize: 13.5, fontWeight: 600 }}>Use your recovery phrase</div>
          </div>
          <div style={{ fontSize: 12.5, color: 'var(--ink-3)', marginBottom: 10, lineHeight: 1.55, paddingLeft: 32 }}>
            The 24 words you wrote down when you signed up. This re-derives your vault key — we never see the phrase.
          </div>
          <div style={{ paddingLeft: 32 }}>
            <BBBtn variant="amber" size="sm" icon={<Ico name="key" size={11} />}>Enter recovery phrase</BBBtn>
          </div>
        </div>

        <div style={{ padding: 14, border: '1px solid var(--line)', borderRadius: 'var(--r-2)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
            <span className="t-mono" style={{
              width: 22, height: 22, borderRadius: 6, background: 'var(--paper-2)', border: '1px solid var(--line-2)',
              color: 'var(--ink-2)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              fontSize: 11, fontWeight: 600,
            }}>2</span>
            <div style={{ fontSize: 13.5, fontWeight: 600 }}>Start fresh</div>
          </div>
          <div style={{ fontSize: 12.5, color: 'var(--ink-3)', lineHeight: 1.55, paddingLeft: 32 }}>
            If you've lost the phrase too, your files are unrecoverable. You can delete this vault and create a new one.
          </div>
        </div>
      </div>

      <div className="t-micro" style={{ color: 'var(--ink-4)', textAlign: 'center' }}>
        This is the cost of real end-to-end encryption. We're sorry it's not what you wanted to read.
      </div>
    </AuthShell>
  );
}

// ─── Session management ─────────────────────────────────────
function HiSessions() {
  const sessions = [
    { device: 'MacBook Pro · Safari', where: 'Berlin · Germany', when: 'Active now', current: true, icon: '💻' },
    { device: 'iPhone 15 · Beebeeb app', where: 'Berlin · Germany', when: '14 min ago', current: false, icon: '📱' },
    { device: 'Pixel 8 · Beebeeb app', where: 'Warsaw · Poland', when: '3 days ago', current: false, icon: '📱' },
    { device: 'Desktop · Windows 11', where: 'Berlin · Germany', when: '6 days ago', current: false, icon: '🖥️' },
    { device: 'Linux · bb CLI v0.4', where: 'Frankfurt · Germany (VPN)', when: '12 days ago', current: false, icon: '🖥️' },
  ];
  return (
    <div style={{ width: 680 }}>
      <div className="bb-card" style={{ padding: 0 }}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
          <div className="t-h3" style={{ fontSize: 14 }}>Active sessions</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>Every device currently holding a valid auth token.</div>
        </div>
        {sessions.map((s, i, arr) => (
          <div key={i} style={{
            padding: '12px 18px',
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
            display: 'flex', alignItems: 'center', gap: 12,
          }}>
            <div style={{
              width: 36, height: 36, borderRadius: 8, background: 'var(--paper-2)',
              border: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center',
              fontSize: 16,
            }}>{s.icon}</div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13.5, fontWeight: 500, display: 'flex', alignItems: 'center', gap: 8 }}>
                {s.device}
                {s.current && <BBChip variant="green" style={{ fontSize: 9.5 }}><span className="dot" /> This device</BBChip>}
              </div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>{s.where} · {s.when}</div>
            </div>
            {!s.current && <BBBtn size="sm" variant="ghost">Revoke</BBBtn>}
          </div>
        ))}
        <div style={{ padding: '12px 18px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)' }}>
          <BBBtn variant="danger" size="sm">Sign out of all other devices</BBBtn>
        </div>
      </div>
    </div>
  );
}

// ─── Account deletion ─────────────────────────────────────
function HiAccountDelete() {
  return (
    <AuthShell title="Delete your account" subtitle="This cannot be undone. Read carefully." width={480}>
      <div style={{
        padding: 14, background: 'oklch(0.97 0.02 25)', border: '1px solid oklch(0.88 0.05 25)',
        borderRadius: 'var(--r-2)', marginBottom: 18,
      }}>
        <div style={{ fontSize: 12.5, color: 'oklch(0.35 0.12 25)', lineHeight: 1.55 }}>
          Within <strong>30 days</strong>, your encrypted blobs are permanently shredded from all three regions. After that there is nothing to recover — not for you, not for us, not for anyone with a court order.
        </div>
      </div>

      <div style={{ marginBottom: 14 }}>
        <div className="t-label" style={{ marginBottom: 8 }}>What gets deleted</div>
        {[
          ['All files and versions', '23.4 GB across 1,247 files'],
          ['All shared links', '12 active — will return 410 Gone'],
          ['Access for team members', '0 people (solo account)'],
          ['Audit logs', 'Retained 12 months per GDPR Art. 17'],
        ].map(([a, b], i) => (
          <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '7px 0', borderBottom: i < 3 ? '1px solid var(--line)' : 'none' }}>
            <Ico name="trash" size={12} color="var(--ink-3)" />
            <span style={{ fontSize: 13, flex: 1 }}>{a}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{b}</span>
          </div>
        ))}
      </div>

      <Field label="Type DELETE to confirm">
        <div className="bb-input">
          <input defaultValue="DELETE" style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }} />
        </div>
      </Field>

      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, marginTop: 14, marginBottom: 16 }}>
        <BBCheck on />
        <div style={{ fontSize: 12, color: 'var(--ink-2)', lineHeight: 1.5 }}>
          I understand my files are encrypted and cannot be recovered after deletion.
        </div>
      </div>

      <div style={{ display: 'flex', gap: 10 }}>
        <BBBtn style={{ flex: 1, justifyContent: 'center' }}>Cancel</BBBtn>
        <BBBtn variant="danger" style={{ flex: 1, justifyContent: 'center', background: 'var(--red)', color: 'var(--paper)', borderColor: 'var(--red)' }}>
          Delete permanently
        </BBBtn>
      </div>
    </AuthShell>
  );
}

Object.assign(window, {
  HiSignup, HiLogin, HiEmailVerify, HiTwoFactor, HiPasskey,
  HiForgotPassword, HiSessions, HiAccountDelete,
});
