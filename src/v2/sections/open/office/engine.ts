// The Office engine on this PC, through Rust only (src-tauri/src/office_v2): its status, the first-time download
// with progress, and the base the window loads the cached files from. The window never fetches the engine itself.
import { listen } from '@tauri-apps/api/event'
import { engineReadyBridge, type OfficeBridge } from '@beebeeb/shared/drive-ui'
import { command } from '../../../../desktopApi'

export interface EngineStatus { state: 'ready' | 'missing' | 'unpublished'; version: string | null; totalBytes: number }
export interface EngineProgress { loaded: number; total: number }

/** The words each failure gets under "We couldn't prepare the editor on this device." */
export function failureDetail(reason: string): string {
  const code = reason.split(':')[0]
  if (code === 'unpublished') return 'This build has no editor bundle to download yet.'
  if (code === 'hash') return 'The editor’s files did not match what this app expects; nothing was kept.'
  if (code === 'host') return 'The download did not finish. Check the connection and try again.'
  if (code === 'disk') return 'There was no room or no access to keep the editor on this PC.'
  if (code === 'boot') return 'The editor downloaded but did not start.'
  return 'Something went wrong while preparing the editor.'
}

export async function engineStatus(): Promise<EngineStatus> {
  const r = await command<EngineStatus>('office_engine_status')
  if (!r.ok) throw new Error(r.reason)
  return r.value
}

/** Download what is missing; `onProgress` follows Rust's `office://progress`. */
export async function prepareEngine(onProgress: (p: EngineProgress) => void): Promise<EngineStatus> {
  const stop = await listen<EngineProgress>('office://progress', (e) => onProgress(e.payload))
  try {
    const r = await command<EngineStatus>('office_engine_prepare')
    if (!r.ok) throw new Error(r.reason)
    return r.value
  } finally {
    stop()
  }
}

/** Where the cached engine is served: the `bboffice` scheme (WebView2 maps it to http://bboffice.localhost). */
export function engineHostUrl(version: string): string {
  const windows = typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)
  return `${windows ? 'http://bboffice.localhost' : 'bboffice://localhost'}/${version}/bb-office-host.html`
}

/** Wait for the engine in the frame to take calls (its bridge and its UNO port), up to `timeoutMs`. */
export function waitForBridge(frame: HTMLIFrameElement, timeoutMs = 180_000): Promise<OfficeBridge> {
  const started = Date.now()
  return new Promise((resolve, reject) => {
    const tick = () => {
      let bridge: OfficeBridge | null = null
      try {
        bridge = frame.contentWindow ? engineReadyBridge(frame.contentWindow) : null
      } catch {
        return reject(new Error('boot: the editor frame is not reachable'))
      }
      if (bridge) return resolve(bridge)
      if (Date.now() - started > timeoutMs) return reject(new Error('boot: timed out'))
      setTimeout(tick, 250)
    }
    tick()
  })
}
