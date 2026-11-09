// The speed test's state, kept outside the pane so leaving Settings doesn't stop it (DW-0786).
// One test at a time; a sign-out or lock clears the figures (clearSpeedTest).
import { useSyncExternalStore } from 'react'
import { command, type SpeedtestResult } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { board } from '../../../board'

export interface SpeedResult { latencyAvg: number; latencyMin: number; latencyMax: number; up: number; down: number }
export type TestState =
  | { phase: 'idle' }
  | { phase: 'running'; step: 'latency' | 'upload' | 'download'; done: number; latency?: SpeedResult['latencyAvg']; latencyRange?: [number, number]; up?: number }
  | { phase: 'done'; result: SpeedResult }
  | { phase: 'error'; message: string }

let state: TestState = { phase: 'idle' }
const subs = new Set<() => void>()
const set = (s: TestState) => { state = s; subs.forEach((f) => f()) }

export const failureSentence = (what: 'ping' | 'upload' | 'download' | 'none') =>
  what === 'ping' ? "Beebeeb didn't answer any of the 5 pings."
    : what === 'upload' ? 'None of the 3 uploads finished.'
      : what === 'download' ? 'None of the 3 downloads finished.'
        : 'The test finished without a speed to report.'

const ALREADY_RUNNING = 'A speed test is already running.'
const COULD_NOT_RUN = 'The speed test couldn’t run.'

export function clearSpeedTest() { set({ phase: 'idle' }) }

/** Bytes per second as the test reports it, in megabits per second (the figure the page shows). */
const mbps = (bps: number) => (bps * 8) / 1_000_000

/**
 * What `run_speedtest` answered, as the page's state (P1c: it answers `{ latency: { min_ms, avg_ms, max_ms },
 * upload_bps, download_bps }`, which the page read as fields it never sends, so every finished test said "None of the
 * 3 uploads finished"). A refusal is Rust's own sentence when it has one (activity_conflict_commands.rs).
 */
export function testStateFrom(r: { ok: true; value: SpeedtestResult } | { ok: false; reason: string }): TestState {
  if (!r.ok) {
    const plain = (t: string) => t.replace(/’/g, "'")
    const said = [failureSentence('ping'), failureSentence('upload'), failureSentence('download'), ALREADY_RUNNING].find((f) => plain(r.reason).startsWith(plain(f)))
    return { phase: 'error', message: said ?? COULD_NOT_RUN }
  }
  const v = r.value
  if (!v.latency || !(v.latency.avg_ms >= 0)) return { phase: 'error', message: failureSentence('ping') }
  if (!(v.upload_bps > 0)) return { phase: 'error', message: failureSentence('upload') }
  if (!(v.download_bps > 0)) return { phase: 'error', message: failureSentence('download') }
  return { phase: 'done', result: { latencyAvg: v.latency.avg_ms, latencyMin: v.latency.min_ms, latencyMax: v.latency.max_ms, up: mbps(v.upload_bps), down: mbps(v.download_bps) } }
}

export function runSpeedTest() {
  if (state.phase === 'running') return
  set({ phase: 'running', step: 'latency', done: 0 })
  // A plain browser on the dev server walks the board's steps (../../../board.ts); a production build has none.
  const walk = hasTauri() ? undefined : board<TestState[]>('speedtestSteps')
  if (walk) {
    walk.forEach((s, i) => setTimeout(() => { if (state.phase === 'running') set(s) }, 900 * (i + 1)))
    return
  }
  void command<SpeedtestResult>('run_speedtest').then((r) => set(testStateFrom(r)))
}

export function useSpeedTest(): TestState {
  return useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => state)
}
