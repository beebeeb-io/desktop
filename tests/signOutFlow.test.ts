import { describe, expect, test } from 'bun:test'
import type { CommandResult, SessionActionOutcome } from '../src/desktopApi'
import type { AskCounts, SignOutCount } from '../src/signOutCopy'
import { SignOutFlow, connectSignOutFlowEvents, connectSignOutFlowMenu, type FlowOutcome, type FlowView, type SignOutFlowApi } from '../src/signOutFlow'

const ok = <T,>(value: T): CommandResult<T> => ({ ok: true, value })
const err = (reason: string): CommandResult<never> => ({ ok: false, reason, unsupported: false })
const asked = (askId: string, over: Partial<SignOutCount> = {}): SignOutCount => ({
  askId, files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [], canSend: true, notSending: null, ...over,
})
const nothing = (askId: string): SignOutCount => asked(askId, { files: 0, copiedFiles: 0, canSend: false })

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}

/** A scripted backend: answers are queued per command; a `deferred` answer waits for the test. */
function harness(script: { count?: Array<CommandResult<SignOutCount> | Promise<CommandResult<SignOutCount>>>; clear?: Array<CommandResult<SessionActionOutcome> | Promise<CommandResult<SessionActionOutcome>>> }) {
  const calls: Array<{ name: string; args?: unknown }> = []
  const views: FlowView[] = []
  const outcomes: FlowOutcome[] = []
  const order: string[] = []
  const counts = [...(script.count ?? [])]
  const clears = [...(script.clear ?? [])]
  const api: SignOutFlowApi = {
    count: async () => { calls.push({ name: 'count' }); return counts.shift()! },
    sendStart: async (askId) => { calls.push({ name: 'sendStart', args: askId }); return ok(undefined) },
    sendStop: async () => { calls.push({ name: 'sendStop' }); return ok(undefined) },
    clearSession: async (keepUnsent) => { calls.push({ name: 'clearSession', args: keepUnsent }); return clears.shift()! },
    acknowledge: async () => { calls.push({ name: 'acknowledge' }); order.push('ack'); return ok(undefined) },
  }
  const flow = new SignOutFlow(api, (view) => views.push(view), (outcome) => { outcomes.push(outcome); order.push('shown') })
  return { flow, calls, views, outcomes, order, names: () => calls.map((c) => c.name) }
}
const tick = () => new Promise((r) => setTimeout(r, 0))

describe('the Settings sign-out flow (spec 2026-10-10 §5.1)', () => {
  /** F1. */
  test('count, send, ask from the phase\'s own snapshot, then sign out with that askId', async () => {
    const h = harness({ count: [ok(asked('first'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names()).toEqual(['count', 'sendStart'])
    expect(h.calls[1].args).toBe('first')
    h.flow.progress(1)
    expect(h.flow.current()).toEqual({ kind: 'sending', changesLeft: 1 })
    // F7 at the flow: a progress event with 0 never renders; the phase ends first.
    h.flow.progress(0)
    expect(h.flow.current()).toEqual({ kind: 'sending', changesLeft: 1 })
    const second: AskCounts = { askId: 'second', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    await h.flow.ended({ reason: 'offline', ask: second })
    expect(h.flow.current()).toEqual({ kind: 'ask', ask: second, reason: 'offline' })
    await h.flow.signOutAtAsk()
    expect(h.calls.at(-1)).toEqual({ name: 'clearSession', args: { askId: 'second' } })
  })

  test('Cancel at the ask calls nothing', async () => {
    const h = harness({ count: [ok(asked('a', { canSend: false, notSending: 'offline' }))] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.cancel()
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    expect(h.names()).toEqual(['count'])
  })

  /** F3, the flow's half: Esc during the phase is "Stop waiting", never Cancel. */
  test('Esc during the send phase stops waiting', async () => {
    const h = harness({ count: [ok(asked('a'))] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.cancel()
    expect(h.flow.current().kind).toBe('sending')
    await h.flow.stopWaiting()
    expect(h.names()).toEqual(['count', 'sendStart', 'sendStop'])
  })

  /** F10. */
  test('a refusal after the phase goes straight to the ask, and the phase starts once', async () => {
    const h = harness({ count: [ok(asked('a')), ok(asked('b', { canSend: true }))], clear: [err('unsent_changes: Some changes…')] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.ended({ reason: 'all_sent', ask: null })
    expect(h.calls.find((c) => c.name === 'clearSession')?.args).toBeUndefined()
    expect(h.flow.current()).toMatchObject({ kind: 'ask', ask: { askId: 'b' } })
    expect(h.names().filter((name) => name === 'sendStart')).toHaveLength(1)
    expect(h.outcomes, 'no toast for the refusal').toEqual([])
  })

  /** PR #130 review, "Reset the send guard when Cancel ends the flow": the guard is per flow (§5.1 step 4), and every
   *  way a flow ends starts the next one fresh. */
  test('Cancel at the ask ends the flow: the next sign-out sends again', async () => {
    const h = harness({ count: [ok(asked('a')), ok(asked('b'))] })
    h.flow.open()
    await h.flow.confirm()
    const ask: AskCounts = { askId: 'c', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    await h.flow.ended({ reason: 'offline', ask })
    h.flow.cancel()
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names(), 'a fresh flow runs its send phase').toEqual(['count', 'sendStart', 'count', 'sendStart'])
    expect(h.flow.current()).toMatchObject({ kind: 'sending' })
  })

  test('a count that fails ends the flow: the next sign-out sends again', async () => {
    const h = harness({ count: [ok(asked('a')), err('unsent_count_failed: …'), ok(asked('b'))] })
    h.flow.open()
    await h.flow.confirm()
    // The phase ended without a snapshot, so the flow counts again, and that count fails.
    await h.flow.ended({ reason: 'stopped', ask: null })
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names()).toEqual(['count', 'sendStart', 'count', 'count', 'sendStart'])
    expect(h.flow.current()).toMatchObject({ kind: 'sending' })
  })

  /** §5.1 "One flow at a time": a second request (the menu, the compact page) while a flow runs is consumed into it
   *  and starts nothing: no second count, no second phase. */
  test('a request while a flow runs starts nothing new', async () => {
    const h = harness({ count: [ok(asked('a')), ok(asked('b'))] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.request()
    expect(h.names(), 'during the phase').toEqual(['count', 'sendStart'])
    const ask: AskCounts = { askId: 'c', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    await h.flow.ended({ reason: 'offline', ask })
    await h.flow.request()
    expect(h.names(), 'at the ask').toEqual(['count', 'sendStart'])
    expect(h.flow.current()).toEqual({ kind: 'ask', ask, reason: 'offline' })
  })

  /** F11. */
  test('a close ends the flow at every step, and late answers do nothing', async () => {
    // The count in flight.
    const late = deferred<CommandResult<SignOutCount>>()
    let h = harness({ count: [late.promise] })
    h.flow.open()
    const counting = h.flow.confirm()
    h.flow.reset()
    late.resolve(ok(asked('a')))
    await counting
    expect(h.names()).toEqual(['count'])
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    // The send phase ends `closed`: no ask, no clear_session. (An answer is scripted, so a flow that wrongly signs out
    // here reaches the clear_session assertion below instead of failing on an unscripted call.)
    h = harness({ count: [ok(asked('a'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.ended({ reason: 'closed', ask: null })
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    expect(h.names()).not.toContain('clearSession')
    // At the ask; then "Sign out…" starts a new flow at the count.
    h = harness({ count: [ok(asked('a', { canSend: false })), ok(nothing('b'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.reset()
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names()).toEqual(['count', 'count', 'clearSession'])
    // A clear_session answer after the close: no toast, no acknowledgement.
    const answer = deferred<CommandResult<SessionActionOutcome>>()
    h = harness({ count: [ok(asked('a', { canSend: false }))], clear: [answer.promise] })
    h.flow.open()
    await h.flow.confirm()
    const signing = h.flow.signOutAtAsk()
    h.flow.reset()
    answer.resolve(err('unsent_copy_failed: Beebeeb couldn’t copy…'))
    await signing
    await tick()
    expect(h.outcomes).toEqual([])
    expect(h.names()).not.toContain('acknowledge')
  })

  /** F12, the Settings caller. */
  test('the result is acknowledged once, after it was shown, and only when it needed showing', async () => {
    let h = harness({ count: [ok(asked('a', { canSend: false }))], clear: [err('Sign-out paused: x')] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.signOutAtAsk()
    await tick()
    expect(h.order).toEqual(['shown', 'ack'])
    h = harness({ count: [ok(nothing('a'))], clear: [ok({ warning: { code: 'finder_removal_unconfirmed', sentence: 'w' } as any })] })
    h.flow.open()
    await h.flow.confirm()
    await tick()
    expect(h.order).toEqual(['shown', 'ack'])
    h = harness({ count: [ok(nothing('a'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    await tick()
    expect(h.names()).not.toContain('acknowledge')
  })

  /** §5.1 step 3: the ask shows only when something waits. A phase can end on any reason but `closed` just after its
   *  last change went up (a "Stop waiting" pressed then ends `stopped`), and Rust's fresh snapshot is then empty: the
   *  flow signs out as it does on `all_sent`, and never shows an ask with nothing in it. */
  test('a phase that ends with an empty snapshot signs out, and shows no empty ask', async () => {
    const h = harness({ count: [ok(asked('a'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    const empty: AskCounts = { askId: 'b', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [] }
    await h.flow.ended({ reason: 'stopped', ask: empty })
    expect(h.views.filter((view) => view.kind === 'ask'), 'no empty ask').toEqual([])
    expect(h.calls.at(-1)).toEqual({ name: 'clearSession', args: undefined })
    expect(h.flow.current()).toEqual({ kind: 'idle' })
  })

  /** F1 and F11 name Rust's events: the progress line follows `sign-out-send-progress`, the ask comes from
   *  `sign-out-send-ended`, and `sign-out-flow-closed` ends the flow. The flow tests above call the flow directly; this
   *  one delivers the events, and the cleanup stops all three listeners. */
  test('the phase\'s events and the close reach the flow, and the cleanup stops listening', async () => {
    const handlers = new Map<string, (event: { payload: any }) => void>()
    const stopped: string[] = []
    const listen = async (name: string, handler: (event: { payload: any }) => void) => {
      handlers.set(name, handler)
      return () => { stopped.push(name) }
    }
    const h = harness({ count: [ok(asked('a', { files: 2, copiedFiles: 2 }))] })
    const cleanup = connectSignOutFlowEvents(listen as any, h.flow)
    h.flow.open()
    await h.flow.confirm()
    handlers.get('sign-out-send-progress')!({ payload: { changesLeft: 1 } })
    expect(h.flow.current()).toEqual({ kind: 'sending', changesLeft: 1 })
    const ask: AskCounts = { askId: 'b', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    handlers.get('sign-out-send-ended')!({ payload: { reason: 'offline', ask } })
    await tick()
    expect(h.flow.current()).toEqual({ kind: 'ask', ask, reason: 'offline' })
    handlers.get('sign-out-flow-closed')!({ payload: null })
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    cleanup()
    await tick()
    expect(stopped.sort()).toEqual(['sign-out-flow-closed', 'sign-out-send-ended', 'sign-out-send-progress'])
  })

  /** F9. */
  test('a flow request is consumed on mount and on the event', async () => {
    const handlers: Array<() => void> = []
    const listen = async (_event: string, handler: () => void) => { handlers.push(handler); return () => {} }
    const answers = [ok(false), ok(true)]
    let requests = 0
    connectSignOutFlowMenu(listen as any, async () => answers.shift()!, () => { requests += 1 })
    await tick()
    expect(requests, 'false changes nothing').toBe(0)
    handlers[0]()
    await tick()
    expect(requests, 'true starts the flow').toBe(1)
  })
})
