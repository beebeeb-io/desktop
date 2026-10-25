/**
 * The speed test's state, kept for the whole window (task 1683 slice 5; PR #140 review). A test moves 60 MB and takes up
 * to 90 seconds, so leaving Settings › About must not lose it: the test goes on in the host, and coming back shows it
 * still running, or the figures it ended with. Same shape as the manual update check (`windows/manualUpdateCheck.ts`).
 *
 * One test at a time: `start` while one runs is a no-op that says nothing (the host refuses a second one as well).
 */
import { subscribeAccountSession } from './accountSession'
import { commandUnavailableLabel, runSpeedtest, type CommandResult, type SpeedtestResult } from './desktopApi'
import { speedTestFailure } from './macSettingsModel'

export type SpeedTestState = { phase: 'idle' } | { phase: 'running' } | { phase: 'done'; result: SpeedtestResult }

export function createSpeedTest(
  run: () => Promise<CommandResult<SpeedtestResult>>,
  /** The account session's changes. The figures belong to the account and the connection they were measured on. */
  subscribeSession: (listener: () => void) => () => void = () => () => {},
) {
  let state: SpeedTestState = { phase: 'idle' }
  /** Moves on every session change: a test that began under an earlier one is not this session's and says nothing. */
  let epoch = 0
  const listeners = new Set<() => void>()
  const publish = (next: SpeedTestState) => {
    state = next
    listeners.forEach((listener) => listener())
  }
  // For the life of the window, as the store is: a sign-out, another account or a lock that ends the session clears the
  // figures. A test still running is not cleared: the host's run holds its one slot until it ends, so the row stays busy
  // until then and only then drops what the old session measured (review P3-2).
  subscribeSession(() => {
    epoch += 1
    if (state.phase === 'done') publish({ phase: 'idle' })
  })
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    /**
     * Runs one test. Resolves to the plain reason when it could not be run (the row then goes back to its hint), and to
     * null when it ran, or when one was already running.
     */
    start: async (): Promise<string | null> => {
      if (state.phase === 'running') return null
      const mine = epoch
      publish({ phase: 'running' })
      // `run` is a `CommandResult` producer: `command()` turns every failure into a result and never rejects.
      const result = await run()
      if (mine !== epoch) {
        publish({ phase: 'idle' })
        return null
      }
      if (!result.ok) {
        publish({ phase: 'idle' })
        return result.unsupported ? commandUnavailableLabel('run_speedtest') : result.reason
      }
      const failure = speedTestFailure(result.value)
      if (failure !== null) {
        publish({ phase: 'idle' })
        return failure
      }
      publish({ phase: 'done', result: result.value })
      return null
    },
  }
}

export const desktopSpeedTest = createSpeedTest(runSpeedtest, subscribeAccountSession)
