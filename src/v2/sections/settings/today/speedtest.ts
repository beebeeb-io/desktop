// The speed test's state, kept outside the pane so leaving Settings doesn't stop it (DW-0786).
// One test at a time; a sign-out or lock clears a finished result (clearSpeedTest).
import { useSyncExternalStore } from 'react'
import { subscribeAccountSession } from '../../../../accountSession'
import { command, type SpeedtestResult } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { board } from '../../../board'

export interface SpeedResult { latencyAvg: number; latencyMin: number; latencyMax: number; up: number; down: number }
export type TestState =
  | { phase: 'idle' }
  | { phase: 'running'; step: 'latency' | 'upload' | 'download'; done: number; latency?: SpeedResult['latencyAvg']; latencyRange?: [number, number]; up?: number; notice?: string }
  | { phase: 'done'; result: SpeedResult }
  | { phase: 'error'; message: string }
  | { phase: 'unavailable'; message: string }

let state: TestState = { phase: 'idle' }
/** Bumped on sign-out, lock, or clear, so a late answer cannot publish the previous session's figures. */
let epoch = 0
const subs = new Set<() => void>()
const set = (s: TestState) => { state = s; subs.forEach((f) => f()) }

export const failureSentence = (what: 'ping' | 'upload' | 'download' | 'none') =>
  what === 'ping' ? "Beebeeb didn't answer any of the 5 pings."
    : what === 'upload' ? 'None of the 3 uploads finished.'
      : what === 'download' ? 'None of the 3 downloads finished.'
        : 'The test finished without a speed to report.'

const ALREADY_RUNNING = 'A speed test is already running.'
const COULD_NOT_RUN = 'The speed test couldn’t run.'
const UNAVAILABLE_MESSAGE = 'The speed test is not available in this build.'
const UNAVAILABLE: TestState = { phase: 'unavailable', message: UNAVAILABLE_MESSAGE }

export function clearSpeedTest() {
  epoch += 1
  if (state.phase !== 'idle') set({ phase: 'idle' })
}

/** `upload_bps` / `download_bps` are bytes per second (bytes moved ÷ seconds), not bits.
 *  The page shows megabits per second: × 8 / 1_000_000. 4_400_000 B/s is 35.2 Mbps. */
const mbps = (bytesPerSecond: number) => (bytesPerSecond * 8) / 1_000_000

const REFUSALS = [failureSentence('ping'), failureSentence('upload'), failureSentence('download'), ALREADY_RUNNING]

/** Rust's sentence when the reason contains one, otherwise a plain sentence with no code. */
function refusalSentence(reason: string): string {
  const plain = (t: string) => t.replace(/’/g, "'")
  const text = plain(reason)
  return REFUSALS.find((sentence) => text.includes(plain(sentence))) ?? COULD_NOT_RUN
}

/**
 * What `run_speedtest` answered. It sends `{ latency: { min_ms, avg_ms, max_ms }, upload_bps, download_bps }`
 * (bytes per second). A missing command is not a speed, and a direction that moved nothing is not 0 Mbps.
 */
export function testStateFrom(r: { ok: true; value: SpeedtestResult } | { ok: false; reason: string; unsupported?: boolean }): TestState {
  if (!r.ok) {
    if (r.unsupported) return UNAVAILABLE
    return { phase: 'error', message: refusalSentence(r.reason) }
  }
  const v = r.value
  const ms = (n: number) => Number.isFinite(n) && n >= 0
  if (!v.latency || !ms(v.latency.min_ms) || !ms(v.latency.avg_ms) || !ms(v.latency.max_ms)) return { phase: 'error', message: failureSentence('ping') }
  if (!(v.upload_bps > 0)) return { phase: 'error', message: failureSentence('upload') }
  if (!(v.download_bps > 0)) return { phase: 'error', message: failureSentence('download') }
  return { phase: 'done', result: { latencyAvg: v.latency.avg_ms, latencyMin: v.latency.min_ms, latencyMax: v.latency.max_ms, up: mbps(v.upload_bps), down: mbps(v.download_bps) } }
}

/** The dev server's plain browser may walk board steps. A production build (`DEV` is not `true`) never does. */
function devSteps(): TestState[] | undefined {
  if (import.meta.env.DEV !== true || hasTauri()) return undefined
  const steps = board<TestState[]>('speedtestSteps')
  return steps && steps.length > 0 ? steps : undefined
}

/** This build can run a test: Tauri has the command, or the dev server is walking its own steps. */
function offeredHere(): boolean {
  return hasTauri() || import.meta.env.DEV === true
}

function view(): TestState {
  if (!offeredHere() && (state.phase === 'idle' || state.phase === 'unavailable')) return UNAVAILABLE
  return state
}

// A finished result belongs to the session that measured it. A run still in the host keeps its row until that
// answer arrives, and the answer is then dropped: the host holds its one slot until the run ends.
subscribeAccountSession(() => {
  if (state.phase === 'running') { epoch += 1; return }
  if (state.phase === 'done' || state.phase === 'error') clearSpeedTest()
})

/** Starts one test. A second call while one runs does not start another and returns the plain sentence. */
export function runSpeedTest(): string | null {
  if (state.phase === 'running') {
    if (state.notice !== ALREADY_RUNNING) set({ ...state, notice: ALREADY_RUNNING })
    return ALREADY_RUNNING
  }
  if (!offeredHere()) {
    if (state.phase !== 'unavailable') set(UNAVAILABLE)
    return UNAVAILABLE_MESSAGE
  }
  const mine = epoch
  set({ phase: 'running', step: 'latency', done: 0 })
  const steps = devSteps()
  if (steps) {
    steps.forEach((step, i) => setTimeout(() => {
      if (mine !== epoch || state.phase !== 'running') return
      set(step)
    }, 900 * (i + 1)))
    return null
  }
  void command<SpeedtestResult>('run_speedtest').then((r) => {
    if (mine !== epoch) {
      if (state.phase === 'running') set({ phase: 'idle' })
      return
    }
    set(testStateFrom(r))
  })
  return null
}

export function useSpeedTest(): TestState {
  return useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, view, view)
}
