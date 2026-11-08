// The speed test's state, kept outside the pane so leaving Settings doesn't stop it (DW-0786).
// One test at a time; a sign-out or lock clears the figures (clearSpeedTest).
import { useSyncExternalStore } from 'react'
import { command } from '../../../../desktopApi'
import { hasTauri } from '../../../source'

export interface SpeedResult { latencyAvg: number; latencyMin: number; latencyMax: number; up: number; down: number }
export type TestState =
  | { phase: 'idle' }
  | { phase: 'running'; step: 'latency' | 'upload' | 'download'; done: number; latency?: SpeedResult['latencyAvg']; latencyRange?: [number, number]; up?: number }
  | { phase: 'done'; result: SpeedResult }
  | { phase: 'error'; message: string }

export const FIXTURE_RESULT: SpeedResult = { latencyAvg: 24, latencyMin: 18, latencyMax: 41, up: 35.2, down: 94.0 }

let state: TestState = { phase: 'idle' }
const subs = new Set<() => void>()
const set = (s: TestState) => { state = s; subs.forEach((f) => f()) }

export const failureSentence = (what: 'ping' | 'upload' | 'download' | 'none') =>
  what === 'ping' ? "Beebeeb didn't answer any of the 5 pings."
    : what === 'upload' ? 'None of the 3 uploads finished.'
      : what === 'download' ? 'None of the 3 downloads finished.'
        : 'The test finished without a speed to report.'

export function clearSpeedTest() { set({ phase: 'idle' }) }

export function runSpeedTest() {
  if (state.phase === 'running') return
  set({ phase: 'running', step: 'latency', done: 0 })
  if (hasTauri()) {
    command<{ latency_ms?: number; latency_min_ms?: number; latency_max_ms?: number; upload_mbps?: number; download_mbps?: number }>('run_speedtest').then((r) => {
      if (!r.ok) return set({ phase: 'error', message: failureSentence('ping') })
      const v = r.value
      if (!v.upload_mbps) return set({ phase: 'error', message: failureSentence('upload') })
      if (!v.download_mbps) return set({ phase: 'error', message: failureSentence('download') })
      set({ phase: 'done', result: { latencyAvg: v.latency_ms ?? 0, latencyMin: v.latency_min_ms ?? v.latency_ms ?? 0, latencyMax: v.latency_max_ms ?? v.latency_ms ?? 0, up: v.upload_mbps, down: v.download_mbps } })
    })
    return
  }
  // No bridge (plain browser): walk the steps with the board's numbers.
  const steps: TestState[] = [
    { phase: 'running', step: 'upload', done: 1, latency: 24, latencyRange: [18, 41] },
    { phase: 'running', step: 'upload', done: 2, latency: 24, latencyRange: [18, 41] },
    { phase: 'running', step: 'download', done: 1, latency: 24, latencyRange: [18, 41], up: 35.2 },
    { phase: 'done', result: FIXTURE_RESULT },
  ]
  steps.forEach((s, i) => setTimeout(() => { if (state.phase === 'running') set(s) }, 900 * (i + 1)))
}

export function useSpeedTest(): TestState {
  return useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => state)
}
