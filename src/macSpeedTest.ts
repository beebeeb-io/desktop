/**
 * The speed test's state, kept for the whole window (task 1683 slice 5; PR #140 review). A test moves 60 MB and takes up
 * to 90 seconds, so leaving Settings › About must not lose it: the test goes on in the host, and coming back shows it
 * still running, or the figures it ended with. Same shape as the manual update check (`windows/manualUpdateCheck.ts`).
 *
 * One test at a time: `start` while one runs is a no-op that says nothing (the host refuses a second one as well).
 */
import { commandUnavailableLabel, runSpeedtest, type CommandResult, type SpeedtestResult } from './desktopApi'
import { speedTestFailure } from './macSettingsModel'

export type SpeedTestState = { phase: 'idle' } | { phase: 'running' } | { phase: 'done'; result: SpeedtestResult }

export function createSpeedTest(run: () => Promise<CommandResult<SpeedtestResult>>) {
  let state: SpeedTestState = { phase: 'idle' }
  const listeners = new Set<() => void>()
  const publish = (next: SpeedTestState) => {
    state = next
    listeners.forEach((listener) => listener())
  }
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
      publish({ phase: 'running' })
      let result: CommandResult<SpeedtestResult>
      try {
        result = await run()
      } catch (error) {
        publish({ phase: 'idle' })
        return error instanceof Error ? error.message : String(error)
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

export const desktopSpeedTest = createSpeedTest(runSpeedtest)
