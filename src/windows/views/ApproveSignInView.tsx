/**
 * ApproveSignInView — approve a sign-in on a new device with the 8-symbol code it shows (no camera).
 * Pending links → type the code → compare the device name → approve (or deny).
 * The vault key is sealed in Rust (core's typed-code approver); this view only drives the steps.
 */

import { useEffect, useState } from 'react'
import {
  deviceApprovalApprove,
  deviceApprovalBegin,
  deviceApprovalDeny,
  deviceApprovalPending,
  type PendingDeviceLink,
} from '../../desktopApi'
import { T, Card, PageHeader, PrimaryBtn } from '../ui'

type Phase =
  | { phase: 'loading' }
  | { phase: 'list'; links: PendingDeviceLink[]; error?: string }
  | { phase: 'code'; link: PendingDeviceLink; error?: string; busy?: boolean }
  | { phase: 'compare'; link: PendingDeviceLink; name: string; error?: string; busy?: boolean }
  | { phase: 'done'; message: string }

function friendly(reason: string): string {
  if (reason.includes('mismatch')) return 'The code did not match. The sign-in was denied; ask the new device to start again.'
  if (reason.includes('invalid_code')) return 'That is not a valid code. It has 8 letters and digits.'
  if (reason.includes('timeout')) return 'The new device did not answer. Make sure it shows the code, then try again.'
  if (reason.includes('not_pending')) return 'This sign-in is no longer waiting.'
  return reason
}

export default function ApproveSignInView({ onClose }: { onClose?: () => void }) {
  const [s, setS] = useState<Phase>({ phase: 'loading' })
  const [code, setCode] = useState('')

  const load = async () => {
    setS({ phase: 'loading' })
    const r = await deviceApprovalPending()
    setS(r.ok ? { phase: 'list', links: r.value } : { phase: 'list', links: [], error: friendly(r.reason) })
  }
  useEffect(() => {
    void load()
  }, [])

  const begin = async (link: PendingDeviceLink) => {
    setS({ phase: 'code', link, busy: true })
    const r = await deviceApprovalBegin(link.id, code)
    setS(r.ok ? { phase: 'compare', link, name: r.value } : { phase: 'code', link, error: friendly(r.reason) })
  }
  const approve = async (link: PendingDeviceLink, name: string) => {
    setS({ phase: 'compare', link, name, busy: true })
    const r = await deviceApprovalApprove(link.id)
    setS(r.ok ? { phase: 'done', message: `${name} is signed in.` } : { phase: 'code', link, error: friendly(r.reason) })
  }
  const deny = async (link: PendingDeviceLink) => {
    await deviceApprovalDeny(link.id)
    setCode('')
    void load()
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Approve a sign-in"
        subtitle="Enter the code shown on the new device, check its name, then approve. Only approve a device you are looking at."
      />
      {s.phase === 'loading' && <div style={{ color: T.ink3 }}>Looking for sign-ins…</div>}

      {s.phase === 'list' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {s.error && <div style={{ color: T.ink2 }}>{s.error}</div>}
          {s.links.length === 0 && !s.error && <div style={{ color: T.ink3 }}>No device is waiting to sign in.</div>}
          {s.links.map((l) => (
            <Card key={l.id}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: 16 }}>
                <span style={{ fontFamily: T.fontMono, color: T.ink }}>{l.device_name}</span>
                <PrimaryBtn onClick={() => { setCode(''); setS({ phase: 'code', link: l }) }}>Enter code</PrimaryBtn>
              </div>
            </Card>
          ))}
          <div><PrimaryBtn onClick={() => void load()}>Refresh</PrimaryBtn></div>
        </div>
      )}

      {s.phase === 'code' && (
        <Card>
          <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div style={{ color: T.ink2 }}>Code shown by <span style={{ fontFamily: T.fontMono }}>{s.link.device_name}</span></div>
            <input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="XXXX-XXXX"
              autoFocus
              spellCheck={false}
              style={{ fontFamily: T.fontMono, fontSize: 20, letterSpacing: 2, padding: 8, background: T.paper2, color: T.ink, border: `1px solid ${T.line}`, borderRadius: 6 }}
            />
            {s.error && <div style={{ color: T.ink2 }}>{s.error}</div>}
            {s.busy ? <div style={{ color: T.ink3 }}>Waiting for the new device…</div> : (
              <div style={{ display: 'flex', gap: 10 }}>
                <PrimaryBtn onClick={() => void begin(s.link)} disabled={code.trim().length === 0}>Continue</PrimaryBtn>
                <PrimaryBtn onClick={() => void deny(s.link)}>Deny</PrimaryBtn>
              </div>
            )}
          </div>
        </Card>
      )}

      {s.phase === 'compare' && (
        <Card>
          <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div style={{ color: T.ink2 }}>The new device calls itself:</div>
            <div style={{ fontFamily: T.fontMono, fontSize: 18, color: T.ink }}>{s.name}</div>
            <div style={{ color: T.ink3 }}>Approve only if this is the name on the screen in front of you.</div>
            {s.error && <div style={{ color: T.ink2 }}>{s.error}</div>}
            <div style={{ display: 'flex', gap: 10 }}>
              <PrimaryBtn onClick={() => void approve(s.link, s.name)} disabled={s.busy}>Approve</PrimaryBtn>
              <PrimaryBtn onClick={() => void deny(s.link)} disabled={s.busy}>Deny</PrimaryBtn>
            </div>
          </div>
        </Card>
      )}

      {s.phase === 'done' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ color: T.ink }}>{s.message}</div>
          {onClose && <div><PrimaryBtn onClick={onClose}>Back to devices</PrimaryBtn></div>}
        </div>
      )}
    </div>
  )
}
