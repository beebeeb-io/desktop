import { expect, test } from 'bun:test'
import { connectNativeUpdateMenu, createManualUpdateCheck } from '../src/windows/manualUpdateCheck'
import { buildUpdateCheckViewModel } from '../src/windows/updateCheckViewModel'
import type { CommandResult, ManualUpdateCheckResult } from '../src/desktopApi'

const failure = { ok: false, reason: 'Could not reach the stable update manifest: connection refused', unsupported: false } as const
const current = { ok: true, value: { status: 'up_to_date', current_version: '0.1.0', channel: 'stable' } } as const
const tick = () => new Promise((resolve) => setTimeout(resolve, 0))
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

function menuHarness(controller: ReturnType<typeof createManualUpdateCheck>, initiallyPending = false) {
  let pending = initiallyPending
  let handler = () => {}
  let consumes = 0
  let unlistens = 0
  const disconnect = connectNativeUpdateMenu(async (callback) => {
    handler = callback
    return () => { unlistens++ }
  }, async () => {
    consumes++
    const value = pending
    pending = false
    return { ok: true, value }
  }, controller.check)
  return {
    click: () => { pending = true; handler() },
    emit: () => handler(),
    disconnect,
    counts: () => ({ consumes, unlistens }),
  }
}

test('native menu failing check leaves Not checked, shows Checking, then Check failed with the actual reason', async () => {
  const result = deferred<CommandResult<ManualUpdateCheckResult>>()
  let commands = 0
  const controller = createManualUpdateCheck(() => { commands++; return result.promise })
  const chips: string[] = []
  const unsubscribe = controller.subscribe(() => chips.push(buildUpdateCheckViewModel(controller.getSnapshot(), '0.1.0', 'stable').chip))
  expect(buildUpdateCheckViewModel(controller.getSnapshot(), '0.1.0', 'stable').chip).toBe('Not checked')
  const menu = menuHarness(controller)
  await tick()
  menu.click()
  await tick()
  expect(chips).toEqual(['Checking'])
  result.resolve(failure)
  await tick()
  expect(commands).toBe(1)
  expect(chips).toEqual(['Checking', 'Check failed'])
  expect(controller.getSnapshot()).toEqual({ kind: 'error', reason: failure.reason })
  menu.disconnect()
  unsubscribe()
})

test('retry after a failed menu check uses the Settings action and reaches up to date', async () => {
  let commands = 0
  const controller = createManualUpdateCheck(async () => ++commands === 1 ? failure : current)
  const menu = menuHarness(controller)
  await tick()
  menu.click()
  await tick()
  expect(controller.getSnapshot().kind).toBe('error')
  await controller.check()
  expect(controller.getSnapshot()).toEqual({ kind: 'up_to_date', currentVersion: '0.1.0', channel: 'stable' })
  expect(commands).toBe(2)
  menu.disconnect()
})

test('native menu works before Settings mounts, and duplicate events do not duplicate the check', async () => {
  let commands = 0
  const controller = createManualUpdateCheck(async () => { commands++; return current })
  const menu = menuHarness(controller, true)
  await tick()
  menu.emit()
  await tick()
  expect(commands).toBe(1)
  expect(controller.getSnapshot().kind).toBe('up_to_date')
  expect(menu.counts().consumes).toBe(2)
  menu.disconnect()
  expect(menu.counts().unlistens).toBe(1)
})

test('menu and Settings clicks during a check coalesce into one IPC command', async () => {
  const result = deferred<CommandResult<ManualUpdateCheckResult>>()
  let commands = 0
  const controller = createManualUpdateCheck(() => { commands++; return result.promise })
  const first = controller.check()
  await controller.check()
  expect(commands).toBe(1)
  result.resolve(current)
  await first
  expect(controller.getSnapshot().kind).toBe('up_to_date')
})

test('changing release channel discards stale results', async () => {
  const result = deferred<CommandResult<ManualUpdateCheckResult>>()
  const controller = createManualUpdateCheck(() => result.promise)
  const first = controller.check()
  controller.reset()
  result.resolve(failure)
  await first
  expect(controller.getSnapshot()).toEqual({ kind: 'idle' })
})

test('a superseded check cannot overwrite a newer channel result', async () => {
  const old = deferred<CommandResult<ManualUpdateCheckResult>>()
  let commands = 0
  const controller = createManualUpdateCheck(() => ++commands === 1 ? old.promise : Promise.resolve(current))
  const first = controller.check()
  controller.reset()
  await controller.check()
  old.resolve(failure)
  await first
  expect(commands).toBe(2)
  expect(controller.getSnapshot().kind).toBe('up_to_date')
})

for (const status of ['update_available', 'downgrade_available'] as const) {
  test(`menu check preserves ${status} handoff`, async () => {
    const controller = createManualUpdateCheck(async () => ({ ok: true, value: {
      status, current_version: '0.1.0', current_channel: 'beta', channel: 'stable', version: '0.2.0', body: 'Notes', release_notes_url: 'https://example.test/notes',
    } }))
    const menu = menuHarness(controller, true)
    await tick()
    expect(controller.getSnapshot().kind).toBe(status)
    menu.disconnect()
  })
}

test('unexpected rejection becomes an honest failure instead of an uncaught promise', async () => {
  const controller = createManualUpdateCheck(async () => { throw new Error('offline') })
  await controller.check()
  expect(controller.getSnapshot()).toEqual({ kind: 'error', reason: 'offline' })
})

test('cleanup during listener registration removes that listener without draining', async () => {
  const registration = deferred<() => void>()
  let unlistens = 0
  let consumes = 0
  const disconnect = connectNativeUpdateMenu(() => registration.promise, async () => {
    consumes++
    return { ok: true, value: true }
  }, async () => { throw new Error('unexpected check') })
  disconnect()
  registration.resolve(() => { unlistens++ })
  await tick()
  expect(unlistens).toBe(1)
  expect(consumes).toBe(0)
})

test('cleanup during pending IPC still fulfills the consumed request once', async () => {
  const consumed = deferred<CommandResult<boolean>>()
  let checks = 0
  const disconnect = connectNativeUpdateMenu(async () => () => {}, () => consumed.promise, async () => { checks++ })
  await tick()
  disconnect()
  consumed.resolve({ ok: true, value: true })
  await tick()
  expect(checks).toBe(1)
})

test('failed check toast displays the reason and its retry drives the same state machine', async () => {
  const { manualUpdateToast } = await import('../src/ManualUpdateFeedback')
  let commands = 0
  const controller = createManualUpdateCheck(async () => ++commands === 1 ? failure : current)
  await controller.check()
  const toast = manualUpdateToast(controller.getSnapshot(), controller.check)
  expect(toast?.variant).toBe('error')
  expect(toast?.title).toBe('Could not check for updates')
  expect(toast?.message).toBe(failure.reason)
  expect(toast?.durationMs).toBeNull()
  expect(toast?.action?.label).toBe('Try again')
  await toast?.action?.onClick()
  expect(commands).toBe(2)
  expect(controller.getSnapshot().kind).toBe('up_to_date')
  expect(manualUpdateToast(controller.getSnapshot(), controller.check)?.variant).toBe('success')
  expect(manualUpdateToast({ kind: 'update_available', version: '0.2.0', currentVersion: '0.1.0', channel: 'stable' }, controller.check)).toBeNull()
})
