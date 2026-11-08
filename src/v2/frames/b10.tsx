// Batch B10's frames (design/desktop-v2/BATCHES.md). Owned by B10 only. Each renders the whole window state against
// the fixtures in ../source/fixtures/auth.ts; the live flow is ../auth/index.tsx.
import type { ReactElement } from 'react'
import type { FrameRegistry } from './types'
import { FrameWindow } from '../chrome/FrameWindow'
import { AnotherDeviceView } from '../auth/AnotherDevice'
import { ApprovalToast, ApproveCodePanel, ApproveDialog, type ApproveDialogProps } from '../auth/Approve'
import { LockedView, NoPlanView } from '../auth/Gate'
import type { SignInProblem } from '../auth/model'
import { ContinueAsView, PasskeyPasswordView, SignInView, type SignInViewProps } from '../auth/SignIn'
import { FirstRunView, LostPhraseView, PhraseView, TwoFactorView } from '../auth/Steps'
import { ANNA, APPROVER, PHRASE_PARTIAL, PHRASE_WRONG, QR_SVG, REQUEST_CODE, REVIEW, SECONDS_LEFT, THIS_PC } from '../source/fixtures/auth'

const noop = () => {}
const PASSWORD = 'correct horse battery'

function signIn(o: Partial<SignInViewProps> = {}): () => ReactElement {
  const props: SignInViewProps = {
    continueAs: null, email: ANNA.email, password: PASSWORD, busy: false, problem: null,
    onEmail: noop, onPassword: noop, onSubmit: noop, onContinueAs: noop, onAnotherDevice: noop, onForgot: noop, onPasskey: noop, onCreate: noop,
    ...o,
  }
  return () => <SignInView {...props} />
}

const problem = (p: SignInProblem, o: Partial<SignInViewProps> = {}) => signIn({ problem: p, ...o })

function device(status: Parameters<typeof AnotherDeviceView>[0]['status'], secondsLeft = SECONDS_LEFT): () => ReactElement {
  return () => (
    <AnotherDeviceView status={status} code={REQUEST_CODE} secondsLeft={secondsLeft} deviceName={THIS_PC} approverName={APPROVER} qrSvg={QR_SVG}
      onCopyCode={noop} onOpenDrive={noop} onAskAgain={noop} onUseEmail={noop} />
  )
}

function approve(stage: ApproveDialogProps['stage'], o: Partial<ApproveDialogProps> = {}): () => ReactElement {
  return () => (
    <FrameWindow section="settings" title="Beebeeb" overlay={
      <ApproveDialog stage={stage} review={REVIEW} typedCode={REQUEST_CODE} password={PASSWORD}
        onTypedCode={noop} onPassword={noop} onApprove={noop} onDeny={noop} onSeeSessions={noop} onDone={noop} {...o} />
    }>
      <ApproveCodePanel code={REQUEST_CODE} onCode={noop} onContinue={noop} />
    </FrameWindow>
  )
}

function twoFactor(mode: 'totp' | 'backup', code: string, state: 'idle' | 'verifying' | 'wrong'): () => ReactElement {
  return () => <TwoFactorView mode={mode} code={code} state={state} onCode={noop} onSubmit={noop} onSwitch={noop} onBack={noop} />
}

export const FRAMES: FrameRegistry = {
  // First run
  'DW-A01': () => <FirstRunView onCreate={noop} onSignIn={noop} />,
  // Sign in
  'DW-A02': signIn({ continueAs: ANNA, password: '' }),
  'DW-A03': signIn({ password: '' }),
  // Continue as
  'DW-A04': () => <ContinueAsView account={ANNA} state="asking" onTryAgain={noop} onAnotherWay={noop} />,
  'DW-A05': () => <ContinueAsView account={ANNA} state="failed" onTryAgain={noop} onAnotherWay={noop} />,
  'DW-A06': () => <ContinueAsView account={ANNA} state="cancelled" onTryAgain={noop} onAnotherWay={noop} />,
  // Another device
  'DW-A07': device('waiting'),
  'DW-A08': device('signed_in'),
  'DW-A09': device('denied'),
  'DW-A10': device('expired', 0),
  // Approving a sign-in
  'DW-A11': () => <ApprovalToast device={APPROVER} country="Germany" />,
  'DW-A12': () => (
    <FrameWindow section="settings" title="Beebeeb"><ApproveCodePanel code="K7QM-4W" onCode={noop} onContinue={noop} /></FrameWindow>
  ),
  'DW-A13': approve('review'),
  'DW-A14': approve('hello'),
  'DW-A15': approve('approved'),
  'DW-A16': approve('denied'),
  'DW-A17': approve('expired'),
  // Email and password
  'DW-A18': signIn({ busy: true }),
  'DW-A19': problem({ kind: 'invalid' }),
  'DW-A20': problem({ kind: 'offline' }),
  'DW-A21': problem({ kind: 'rate-limited', seconds: 30 }, { waitSeconds: 30 }),
  'DW-A22': problem({ kind: 'locked-hint' }),
  'DW-A63': problem({ kind: 'deleted' }),
  'DW-A23': () => <PhraseView words={PHRASE_PARTIAL} focus={4} unknown={[]} state="idle" onWords={noop} onRestore={noop} onApproveInstead={noop} onLost={noop} />,
  'DW-A24': () => <PhraseView words={PHRASE_WRONG} unknown={[2]} state="wrong" onWords={noop} onRestore={noop} onApproveInstead={noop} onLost={noop} />,
  'DW-A25': () => <LostPhraseView onFound={noop} onCancelBilling={noop} onErase={noop} onDelete={noop} />,
  // Two-factor and passkeys
  'DW-A26': twoFactor('totp', '4821', 'idle'),
  'DW-A27': twoFactor('totp', '', 'wrong'),
  'DW-A28': twoFactor('backup', '', 'idle'),
  'DW-A29': signIn({ password: '', passkeyPrompt: true }),
  'DW-A30': () => <PasskeyPasswordView email={ANNA.email} password="" busy={false} onPassword={noop} onUnlock={noop} />,
  // Before Files: locked, no plan yet
  'DW-0009': () => <LockedView prompt onUnlock={noop} />,
  'DW-0014': () => <NoPlanView onChoose={noop} onCheckAgain={noop} onSignOut={noop} />,
}
