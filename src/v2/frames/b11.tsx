// Batch B11's frames (design/desktop-v2/BATCHES.md). Owned by B11 only. Each renders the whole window state against the
// fixtures in ../source/fixtures/signup.ts; the live flows are ../auth/create, ../auth/forgot, ../onboarding, ../guide.
import type { ReactElement } from 'react'
import type { FrameRegistry } from './types'
import { FRAMES as FILES_FRAMES } from './b04'
import { FRAMES as SHARED_FRAMES } from './b05'
import { FRAMES as PHOTOS_FRAMES } from './b06'
import {
  ConfirmStep, CreatingStep, DeviceStep, DoneStep as AccountReady, EmailStep, PasswordStep, PhraseStep, TermsStep, VerifyStep,
} from '../auth/create/views'
import { NeitherView, NewPasswordView, OptionsView, PasswordUpdatedView, RecoverPhraseView } from '../auth/forgot'
import { InstallerScene } from '../onboarding/Installer'
import {
  BackupStep, CompareView, DoneStep, ExplorerStep, LockStep, LoginStep, NotificationsStep, PasskeyStep, PhotosStep, SetupWindow,
  type ExplorerStatus, type SetupStep,
} from '../onboarding/views'
import { DEFAULT_ON, toBackupFolders } from '../onboarding'
import { GuideTip, TrayTipScene } from '../guide/Guide'
import {
  DEVICE, EMAIL, EXPLORER_ERROR, KNOWN_FOLDERS, METER_GOOD, METER_SHORT, METER_STRONG, POLICY, POSITIONS, RECOVER_PARTIAL,
  RECOVER_WRONG, SAMPLE, SUMMARY, WORDS,
} from '../source/fixtures/signup'
import type { PasswordMeter } from '../auth/create/api'

const noop = () => {}
const PW = 'Winter-harbour-lantern-42'

function verify(o: Partial<Parameters<typeof VerifyStep>[0]> = {}): () => ReactElement {
  return () => (
    <VerifyStep email={EMAIL} code="" length={POLICY.code_length} secondsLeft={42} state="idle" notice={null}
      onCode={noop} onVerify={noop} onResend={noop} onDifferentEmail={noop} {...o} />
  )
}

function password(meter: PasswordMeter, o: Partial<Parameters<typeof PasswordStep>[0]> = {}): () => ReactElement {
  return () => (
    <PasswordStep password={PW.slice(0, meter.length)} confirm="" meter={meter} breach={null} blocked={false} show={false} busy={false}
      canContinue={false} onPassword={noop} onConfirm={noop} onShow={noop} onBlur={noop} onContinue={noop} {...o} />
  )
}

function confirmWords(answers: string[], wrong: boolean, suggestions: string[] = []): () => ReactElement {
  return () => (
    <ConfirmStep positions={POSITIONS} answers={answers} suggestions={suggestions} active={suggestions.length ? 2 : 0} wrong={wrong} busy={false}
      onAnswer={noop} onActive={noop} onShowAgain={noop} onConfirm={noop} />
  )
}

function setup(step: SetupStep | 'done' | 'compare', body: ReactElement, explorer: ExplorerStatus = 'ready'): () => ReactElement {
  return () => <SetupWindow step={step} explorer={explorer} onSkip={step === 'done' || step === 'compare' ? undefined : noop}>{body}</SetupWindow>
}

/** The guide's tips sit over the real window (DW-0301 to DW-0305): the window each tip shows is that section's own frame. */
const BACKDROP: Record<'files' | 'shared' | 'photos', () => ReactElement> = {
  files: () => FILES_FRAMES['DW-1101']!(),
  shared: () => SHARED_FRAMES['DW-0501']!(),
  photos: () => PHOTOS_FRAMES['DW-0601']!(),
}

function Tip({ index, section }: { index: number; section: 'files' | 'shared' | 'photos' }) {
  return <>{BACKDROP[section]()}<GuideTip index={index} onNext={noop} onSkip={noop} /></>
}

export const FRAMES: FrameRegistry = {
  // Create an account
  'DW-A31': () => <EmailStep email="" busy={false} error={null} onEmail={noop} onSend={noop} onBack={noop} />,
  'DW-A32': verify({ code: '4821' }),
  'DW-A33': verify({ code: '48210937', state: 'wrong', secondsLeft: 31 }),
  'DW-A34': verify({ notice: { tone: 'neutral', text: 'If this address can be used, a new email is on its way.' } }),
  'DW-A35': () => <TermsStep termsUrl={POLICY.terms_url!} privacyUrl={POLICY.privacy_url!} accepted understood={false} onAccepted={noop} onUnderstood={noop} onContinue={noop} onOpen={noop} />,
  'DW-A36': password(METER_STRONG, { password: PW, confirm: PW, canContinue: true }),
  'DW-A37': password(METER_SHORT),
  'DW-A38': password(METER_GOOD, { password: 'Password123456789012345', breach: 'breached' }),
  'DW-A39': () => <PhraseStep words={WORDS} saved={false} copied={false} onSaved={noop} onCopy={noop} onSave={noop} onPrint={noop} onContinue={noop} />,
  'DW-A40': confirmWords(['thank', 'worth', 'tha'], false, ['thank', 'that', 'theme']),
  'DW-A41': confirmWords(['thank', 'wave', 'thank'], true),
  'DW-A42': () => <DeviceStep name={DEVICE} onName={noop} onCreate={noop} />,
  'DW-A43': () => <CreatingStep problem={null} onTryAgain={noop} onSignIn={noop} />,
  'DW-A44': () => <AccountReady onContinue={noop} />,
  // Forgot password
  'DW-A45': () => <OptionsView onPhrase={noop} onDevice={noop} onNeither={noop} onBack={noop} />,
  'DW-A46': () => <NeitherView onBack={noop} />,
  'DW-A47': () => <RecoverPhraseView email={EMAIL} words={RECOVER_PARTIAL} focus={8} unknown={[]} state="idle" onEmail={noop} onWords={noop} onVerify={noop} onLost={noop} />,
  'DW-A48': () => <RecoverPhraseView email={EMAIL} words={RECOVER_WRONG} unknown={[5]} state="wrong" onEmail={noop} onWords={noop} onVerify={noop} onLost={noop} />,
  'DW-A49': () => (
    <NewPasswordView password={PW} confirm={PW} meter={METER_STRONG} breach={null} show={false} busy={false} canSet problem={null}
      onPassword={noop} onConfirm={noop} onShow={noop} onBlur={noop} onSet={noop} />
  ),
  'DW-A50': () => <PasswordUpdatedView signInNext={false} onContinue={noop} />,
  // Install Beebeeb on Windows
  'DW-1901': () => <InstallerScene page="installing" />,
  'DW-1902': () => <InstallerScene page="finish" />,
  // First run on Windows
  'DW-0201': setup('passkey', <PasskeyStep asking={false} password="" error={null} onPassword={noop} onAdd={noop} onNotNow={noop} />, 'adding'),
  'DW-0202': setup('lock', <LockStep on after="Immediately" onToggle={noop} onAfter={noop} onContinue={noop} onNotNow={noop} />, 'adding'),
  'DW-0203': setup('backup', <BackupStep folders={toBackupFolders(KNOWN_FOLDERS, DEFAULT_ON)} busy={false} error={null} onToggle={noop} onAddFolder={noop} onTurnOn={noop} onNotNow={noop} />),
  'DW-0204': setup('photos', <PhotosStep sample={SAMPLE} size="4K" format="HEIC" onSize={noop} onFormat={noop} onCompare={noop} onContinue={noop} onNotNow={noop} />),
  'DW-0206': setup('explorer', <ExplorerStep status="ready" errorCode={null} onShow={noop} onContinue={noop} onTryAgain={noop} />),
  'DW-0206b': setup('explorer', <ExplorerStep status="adding" errorCode={null} onShow={noop} onContinue={noop} onTryAgain={noop} />, 'adding'),
  'DW-0280': setup('explorer', <ExplorerStep status="failed" errorCode={EXPLORER_ERROR} onShow={noop} onContinue={noop} onTryAgain={noop} />, 'failed'),
  'DW-0207': setup('notifications', <NotificationsStep onAllow={noop} onNotNow={noop} />),
  'DW-0281': setup('login', <LoginStep on onToggle={noop} onContinue={noop} />),
  'DW-0208': setup('done', <DoneStep summary={SUMMARY} onRow={noop} onOpen={noop} />),
  'DW-0209': setup('compare', <CompareView mode="Stacked" onMode={noop} onChoose={noop} onDone={noop} />),
  // The first-time guide
  'DW-0380': () => <TrayTipScene window={BACKDROP.files()} onGotIt={noop} />,
  'DW-0301': () => <Tip index={0} section="files" />,
  'DW-0302': () => <Tip index={1} section="files" />,
  'DW-0303': () => <Tip index={2} section="shared" />,
  'DW-0304': () => <Tip index={3} section="photos" />,
  'DW-0305': () => <Tip index={4} section="files" />,
}

export const B11_FRAME_COUNT = 39
