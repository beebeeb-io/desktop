/**
 * Spec 2026-10-10 §5.1 (task 1887): the sign-out flow in Settings › Account. Rust owns the snapshot, the phase and
 * its deadline; this owns only what the sheet shows. A close or reload (`sign-out-flow-closed`) resets it, and any
 * answer that arrives after a reset is dropped: no toast, no acknowledgement, because Rust shows what needs showing.
 */
import { clearSession, command, type CommandResult, type SessionActionOutcome } from './desktopApi'
import type { AskCounts, EndReason, NotSending, SendEnded, SignOutCount } from './signOutCopy'

export type FlowView =
  | { kind: 'idle' }
  | { kind: 'confirm' }
  | { kind: 'counting' }
  | { kind: 'sending'; changesLeft: number }
  | { kind: 'ask'; ask: AskCounts; reason: EndReason | NotSending | null }
  | { kind: 'signingOut' }

export type FlowOutcome = { kind: 'signedOut'; warning: string | null } | { kind: 'failed'; message: string }

export interface SignOutFlowApi {
  count(): Promise<CommandResult<SignOutCount>>
  sendStart(askId: string): Promise<CommandResult<void>>
  sendStop(): Promise<CommandResult<void>>
  clearSession(keepUnsent?: { askId: string }): Promise<CommandResult<SessionActionOutcome>>
  acknowledge(): Promise<CommandResult<void>>
}

export const signOutFlowApi: SignOutFlowApi = {
  count: () => command<SignOutCount>('sign_out_count'),
  sendStart: (askId) => command<void>('sign_out_send_start', { askId }),
  sendStop: () => command<void>('sign_out_send_stop'),
  clearSession: (keepUnsent) => clearSession({ keepUnsent }),
  acknowledge: () => command<void>('sign_out_result_ack'),
}

const REFUSED = 'unsent_changes: '

export class SignOutFlow {
  private view: FlowView = { kind: 'idle' }
  private generation = 0
  private sent = false

  constructor(
    private readonly api: SignOutFlowApi,
    private readonly show: (view: FlowView) => void,
    private readonly report: (outcome: FlowOutcome) => void,
  ) {}

  current(): FlowView {
    return this.view
  }

  private set(view: FlowView) {
    this.view = view
    this.show(view)
  }

  /** "Sign out…". */
  open(): void {
    if (this.view.kind === 'idle') this.set({ kind: 'confirm' })
  }

  /** The confirm sheet's "Sign out". */
  async confirm(): Promise<void> {
    if (this.view.kind === 'confirm') await this.count()
  }

  /** A flow request from the menu or the compact page: straight to the count. */
  async request(): Promise<void> {
    if (this.view.kind === 'idle' || this.view.kind === 'confirm') await this.count()
  }

  /** Cancel, or Esc at the confirm and at the ask. Esc during the phase is `stopWaiting`, never this. */
  cancel(): void {
    if (this.view.kind === 'confirm' || this.view.kind === 'ask') this.set({ kind: 'idle' })
  }

  async stopWaiting(): Promise<void> {
    if (this.view.kind === 'sending') await this.api.sendStop()
  }

  progress(changesLeft: number): void {
    if (this.view.kind === 'sending' && changesLeft > 0) this.set({ kind: 'sending', changesLeft })
  }

  async ended(event: SendEnded): Promise<void> {
    if (this.view.kind !== 'sending') return
    if (event.reason === 'closed') return this.reset()
    if (event.reason === 'all_sent') return this.signOut(undefined)
    // §5.1 step 3: the ask shows only when something waits. A phase that ended just after its last change went up
    // carries an empty snapshot; that is all sent too.
    if (event.ask && event.ask.files === 0 && event.ask.others === 0) return this.signOut(undefined)
    if (event.ask) return this.set({ kind: 'ask', ask: event.ask, reason: event.reason === 'stopped' ? null : event.reason })
    return this.count()
  }

  /** The ask's "Sign out". */
  async signOutAtAsk(): Promise<void> {
    if (this.view.kind === 'ask') await this.signOut({ askId: this.view.ask.askId })
  }

  /** `sign-out-flow-closed`, or a phase that ended `closed`. */
  reset(): void {
    this.generation += 1
    this.sent = false
    this.set({ kind: 'idle' })
  }

  private async count(): Promise<void> {
    const generation = this.generation
    this.set({ kind: 'counting' })
    const counted = await this.api.count()
    if (generation !== this.generation) return
    if (!counted.ok) {
      this.set({ kind: 'idle' })
      this.report({ kind: 'failed', message: counted.reason })
      return
    }
    const ask = counted.value
    if (ask.files === 0 && ask.others === 0) return this.signOut(undefined)
    // §5.1 step 4: a phase starts at most once per flow, so an autosaving editor cannot loop the person.
    if (ask.canSend && !this.sent) {
      this.sent = true
      this.set({ kind: 'sending', changesLeft: ask.files + ask.others })
      const started = await this.api.sendStart(ask.askId)
      if (generation !== this.generation) return
      if (!started.ok) return this.count()
      return
    }
    this.set({ kind: 'ask', ask, reason: ask.notSending })
  }

  private async signOut(keepUnsent: { askId: string } | undefined): Promise<void> {
    const generation = this.generation
    this.set({ kind: 'signingOut' })
    const result = await this.api.clearSession(keepUnsent)
    if (generation !== this.generation) return
    if (!result.ok && result.reason.startsWith(REFUSED)) return this.count()
    this.sent = false
    this.set({ kind: 'idle' })
    const outcome: FlowOutcome = result.ok
      ? { kind: 'signedOut', warning: result.value.warning?.sentence ?? null }
      : { kind: 'failed', message: result.reason }
    this.report(outcome)
    // F12: after it was shown, and only for what Rust holds pending.
    if (outcome.kind === 'failed' || outcome.warning !== null) void this.api.acknowledge()
  }
}

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>

/** The phase's events and the close, for the Settings window's root, which owns the flow (F9). Returns the cleanup. */
export function connectSignOutFlowEvents(listen: Listen, flow: SignOutFlow): () => void {
  const unlistens = [
    listen<{ changesLeft: number }>('sign-out-send-progress', (event) => flow.progress(event.payload.changesLeft)),
    listen<SendEnded>('sign-out-send-ended', (event) => void flow.ended(event.payload)),
    listen<null>('sign-out-flow-closed', () => flow.reset()),
  ]
  return () => {
    for (const unlisten of unlistens) void unlisten.then((stop) => stop())
  }
}

/** F9: consume a pending request on mount and on every `menu:sign-out-flow`. */
export function connectSignOutFlowMenu(listen: Listen, consume: () => Promise<CommandResult<boolean>>, onRequest: () => void): () => void {
  const take = async () => {
    const result = await consume()
    if (result.ok && result.value) onRequest()
  }
  void take()
  const unlisten = listen<null>('menu:sign-out-flow', () => void take())
  return () => void unlisten.then((stop) => stop())
}
