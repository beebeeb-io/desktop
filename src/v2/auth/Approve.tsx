// DW-A11 to A17: approving a sign-in from this PC. The notification (Windows draws it), the code field in Account,
// the review dialog (what the server measured, what the device reported, the typed code), Windows Hello, and the
// results. The approval itself runs in Rust (`auth_approve_request`): Windows Hello, then the step-up, claim and relay.
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { BBProductIcon, Icon } from '@beebeeb/shared/design'
import { approveLookup, approveRequest, type ApprovalReview } from './api'
import { approveErrorText, countryName, formatDeviceCode, isDeviceCode, type ApproveStage } from './model'
import { Field, HelloGlyph, HelloPrompt, Note, PrimaryButton, SecondaryButton, TextLink, Tile } from './parts'

/** DW-A11: the toast Windows draws bottom-right over whatever is open. Only its content is the app's. */
export function ApprovalToast({ device, country }: { device: string; country: string }) {
  return (
    <div className="relative h-screen overflow-hidden bg-win-desktop-2 font-sans" data-desktop>
      <div className="absolute bottom-[60px] right-3 w-[364px] rounded-lg border border-win-line-soft bg-win-title p-4 text-win-ink-strong shadow-xl" role="status" data-toast>
        <div className="mb-2 flex items-center gap-2 text-[12px] text-win-ink-3"><BBProductIcon product="drive" size={16} />Beebeeb Drive</div>
        <div className="text-[14px] font-semibold">Allow Drive on {device}?</div>
        <div className="mt-1 text-[13px] leading-[18px]">
          Someone in {country} asked to sign in to your account. If this wasn’t you, don’t approve it.
        </div>
      </div>
      <div className="absolute inset-x-0 bottom-0 h-12 border-t border-win-line-accent bg-win-tab" aria-hidden />
    </div>
  )
}

/** DW-A12: Settings › Security › Approve a sign-in, in the settings view. The PC types the code; it does not scan. */
export function ApproveCodePanel({ code, error, busy, back, onCode, onContinue }: {
  code: string; error?: string; busy?: boolean
  /** The settings page's way back (Sign-in and security), as its other pages have it. */
  back?: ReactNode
  onCode: (v: string) => void; onContinue: () => void
}) {
  return (
    <section aria-labelledby="approve-heading" className="mx-auto w-full max-w-[640px] px-8 py-6" data-approve-panel>
      {back}
      <h1 id="approve-heading" className="m-0 text-[20px] font-semibold leading-7 text-ink">Approve a sign-in</h1>
      <p className="mt-1 mb-4 text-[13px] leading-[18px] text-ink-2">
        Type the code the other device is showing. Only approve a request you started yourself.
      </p>
      <form onSubmit={(e) => { e.preventDefault(); if (isDeviceCode(code) && !busy) onContinue() }} noValidate>
        <div className="max-w-[320px]">
          <Field label="Code on the other device" mono placeholder="XXXX-XXXX" autoComplete="off" spellCheck={false} autoCorrect="off" value={code}
            error={error} aria-describedby={error ? undefined : 'approve-code-hint'} onChange={(e) => onCode(formatDeviceCode(e.target.value))} />
          {!error && <p id="approve-code-hint" className="-mt-1 mb-3 text-[12px] text-ink-3">Eight characters, shown as XXXX-XXXX.</p>}
        </div>
        <div className="flex items-center gap-3">
          <div className="w-[160px]">
            <PrimaryButton type="submit" disabled={!isDeviceCode(code) || busy}><Icon name="check" size={14} />{busy ? 'Looking it up…' : 'Continue'}</PrimaryButton>
          </div>
          <span className="text-[12px] text-ink-3">or scan its QR code with your iPhone instead</span>
        </div>
      </form>
    </section>
  )
}

function FactRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex justify-between gap-4 py-1 text-[12px]" data-fact-row>
      <dt className="text-ink-3">{label}</dt>
      <dd className="m-0 text-right text-ink">{children}</dd>
    </div>
  )
}

function timeOf(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
}

export interface ApproveDialogProps {
  stage: ApproveStage
  review: ApprovalReview
  typedCode: string
  password: string
  unusualPlace?: boolean
  error?: string
  busy?: boolean
  onTypedCode: (v: string) => void
  onPassword: (v: string) => void
  onApprove: () => void
  onDeny: () => void
  onSeeSessions: () => void
  onDone: () => void
}

/** DW-A13 to A17 as one dialog over the window. */
export function ApproveDialog(p: ApproveDialogProps) {
  const device = p.review.device_name ?? 'that device'
  return (
    <div className="absolute inset-0 z-40 grid place-items-center bg-scrim">
      <div role="dialog" aria-modal="true" aria-labelledby="approve-dialog-title" className="w-[440px] rounded-xl border border-line bg-paper p-6 shadow-2xl" data-approve-dialog={p.stage}>
        {p.stage === 'review' || p.stage === 'hello' ? <Review {...p} device={device} /> : <Result {...p} device={device} />}
      </div>
      {p.stage === 'hello' && <HelloPrompt reason={`Approve Drive on ${device}`} />}
    </div>
  )
}

function Review(p: ApproveDialogProps & { device: string }) {
  const matches = formatDeviceCode(p.typedCode) === p.review.code
  const { onDeny, busy } = p
  // Esc is Deny (DW-A13), and only Deny: taken in the capture phase so the settings view's Esc (Back to Drive) does not
  // also run. Enter approves only when the code matches (the form's submit guard).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      if (!busy) onDeny()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onDeny, busy])
  // Rust answers a country code; the board says the country's name (M3b's leftovers).
  const country = countryName(p.review.country)
  return (
    <form
      onSubmit={(e) => { e.preventDefault(); if (matches && p.password && !p.busy) p.onApprove() }}
      noValidate
    >
      <h2 id="approve-dialog-title" className="m-0 text-[17px] font-semibold text-ink">Allow Drive on {p.device}?</h2>
      <p className="mt-1 mb-4 text-[12px] text-ink-2">Approving gives that device the keys to your files.</p>
      {p.unusualPlace && <Note tone="red" title="Unusual place">Your account has never signed in from {country} before.</Note>}
      {p.stage === 'hello' && <Note tone="amber">Approving shares your keys with that device. They travel encrypted, and Beebeeb can’t read them.</Note>}
      <div className="mb-3 rounded-md border border-line px-3 py-2">
        <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-3">Measured by Beebeeb</div>
        <dl className="m-0">
          <FactRow label="Country">{country}</FactRow>
          <FactRow label="Network address"><span className="font-mono">{p.review.ip}</span></FactRow>
          <FactRow label="Time of request"><span className="font-mono">{timeOf(p.review.requested_at)}</span></FactRow>
        </dl>
      </div>
      <div className="mb-4 rounded-md border border-line px-3 py-2">
        <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-3">Reported by that device · not verified</div>
        <dl className="m-0">
          <FactRow label="Name">{p.review.device_name ?? 'Not reported'}</FactRow>
          <FactRow label="App">{[p.review.client, p.review.os].filter(Boolean).join(' on ') || 'Not reported'}</FactRow>
        </dl>
      </div>
      <Field label="Code shown on that device" mono placeholder="XXXX-XXXX" autoComplete="off" spellCheck={false} autoCorrect="off"
        value={p.typedCode} onChange={(e) => p.onTypedCode(formatDeviceCode(e.target.value))} disabled={p.busy} />
      <Field label="Your password" secret value={p.password} onChange={(e) => p.onPassword(e.target.value)} disabled={p.busy}
        error={p.error ? approveErrorText(p.error) : undefined} />
      <div className="mt-1 flex gap-2.5">
        <SecondaryButton onClick={p.onDeny} disabled={p.busy}>Deny</SecondaryButton>
        <PrimaryButton type="submit" disabled={!matches || !p.password || p.busy}><HelloGlyph size={15} />Approve</PrimaryButton>
      </div>
      <p className="mt-2 text-center text-[11px] text-ink-3">Approve asks for Windows Hello.</p>
    </form>
  )
}

function Result(p: ApproveDialogProps & { device: string }) {
  if (p.stage === 'approved') {
    return (
      <>
        <Tile icon="check" tone="green" />
        <h2 id="approve-dialog-title" className="m-0 text-[17px] font-semibold text-ink">Approved</h2>
        <ul className="my-4 list-none rounded-md border border-line p-0" data-session-list>
          <li className="flex items-center justify-between px-3 py-2.5 text-[13px]">
            <span className="text-ink">{p.device}</span>
            <span className="text-[12px] text-ink-3">New session · <span className="font-mono">just now</span></span>
          </li>
        </ul>
        <TextLink onClick={p.onSeeSessions}>See devices and sessions</TextLink>
        <div className="mt-5"><SecondaryButton onClick={p.onDone}>Done</SecondaryButton></div>
      </>
    )
  }
  if (p.stage === 'denied') {
    return (
      <>
        <Tile icon="x" tone="neutral" />
        <h2 id="approve-dialog-title" className="m-0 text-[17px] font-semibold text-ink">Denied</h2>
        <p className="mt-1 mb-4 text-[13px] text-ink-2">Nothing was signed in and no keys were shared.</p>
        <Note tone="red">If you didn’t start this, change your password and check Devices and sessions.</Note>
        <PrimaryButton amber={false} onClick={p.onSeeSessions}>Check devices and sessions</PrimaryButton>
      </>
    )
  }
  return (
    <>
      <Tile icon="clock" tone="neutral" />
      <h2 id="approve-dialog-title" className="m-0 text-[17px] font-semibold text-ink">
        {p.stage === 'already-handled' ? 'Already handled' : 'This request expired'}
      </h2>
      <p className="mt-1 mb-5 text-[13px] text-ink-2">
        {p.stage === 'already-handled'
          ? 'Another of your devices answered this request first. Nothing was shared from this PC.'
          : 'Nothing was approved and no keys were shared. Ask for a new code on the other device if you still want to sign in.'}
      </p>
      <PrimaryButton amber={false} onClick={p.onDone}>Done</PrimaryButton>
    </>
  )
}

/**
 * The live approval, for Settings › Security › Approve a sign-in (mounted by the Security pane): look the code up,
 * review, then Rust raises Windows Hello and runs the step-up, claim and relay. The password is cleared from state as
 * soon as it is sent. Done leaves for Account (DW-A17); the result screens' sessions link opens Devices (DW-A55).
 */
export function ApproveSignIn({ back, onBack, onSeeSessions, onDone }: {
  /** The page's way back (a link), and what Esc does on the code and result screens: back to Sign-in and security. */
  back?: ReactNode; onBack?: () => void; onSeeSessions: () => void; onDone: () => void
}) {
  const [code, setCode] = useState('')
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A12: the board's one honest message sits under the code field, tied to it.
  const [lookupError, setLookupError] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [review, setReview] = useState<ApprovalReview | null>(null)
  const [stage, setStage] = useState<ApproveStage>('code')
  const [typed, setTyped] = useState('')
  const [password, setPassword] = useState('')
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A13: errors are announced as alerts tied to the password field.
  const [error, setError] = useState<string>()
  useEffect(() => () => setPassword(''), [])

  const lookup = async () => {
    setBusy(true)
    setLookupError(undefined)
    const r = await approveLookup(code)
    setBusy(false)
    if (!r.ok) { setLookupError(approveErrorText(r.reason)); return }
    setReview(r.value)
    setTyped('')
    setStage(r.value.seconds_left <= 0 ? 'expired' : 'review')
  }
  // One answer per request: a second Esc, or Esc and a click, while the first is on its way sends nothing (a second deny
  // would come back already_handled and say another device answered).
  const deciding = useRef(false)
  const decide = async (approve: boolean) => {
    if (deciding.current) return
    deciding.current = true
    const secret = password
    setPassword('')
    setError(undefined)
    setBusy(true)
    if (approve) setStage('hello')
    const r = await approveRequest(review?.code ?? code, secret, approve)
    deciding.current = false
    setBusy(false)
    if (r.ok) { setStage(r.value.status === 'approved' ? 'approved' : 'denied'); return }
    if (r.reason === 'already_handled') setStage('already-handled')
    else if (r.reason === 'expired' || r.reason === 'request_not_found') setStage('expired')
    else if (r.reason === 'hello_cancelled') setStage('review')
    else { setStage('review'); setError(r.reason) }
  }
  const done = () => { setReview(null); setStage('code'); setCode(''); onDone() }
  // Esc goes back to Sign-in and security, as on the other Security pages; the review takes its own Esc (Deny).
  useEffect(() => {
    if (!onBack || stage === 'review' || stage === 'hello') return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return
      e.preventDefault()
      e.stopPropagation()
      onBack()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onBack, stage])
  return (
    <>
      <ApproveCodePanel code={code} error={lookupError} busy={busy && stage === 'code'} back={back} onCode={setCode} onContinue={() => void lookup()} />
      {review && stage !== 'code' && (
        <ApproveDialog
          stage={stage} review={review} typedCode={typed} password={password} error={error} busy={busy}
          onTypedCode={setTyped} onPassword={setPassword}
          onApprove={() => void decide(true)} onDeny={() => void decide(false)}
          onSeeSessions={onSeeSessions} onDone={done}
        />
      )}
    </>
  )
}
