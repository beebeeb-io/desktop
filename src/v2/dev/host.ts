// Dev only (batch M2), never bundled: in a plain browser there is no Rust behind `invoke`. Tauri's own client calls
// `window.__TAURI_INTERNALS__.invoke`, so the v2 dev server puts a fixture host there: it answers the reads the board's
// fixtures hold (./reads.ts) and refuses everything else as "not implemented", which desktopApi's command() reports as
// unavailable, exactly as a browser without the host did. No production module changes: `hasTauri()` (../source) does
// not count this host as Tauri, and it carries no window metadata, so window and webview calls still fail as before.
import type { CommandResult } from '../../desktopApi'

export type Answer = (name: string, args?: Record<string, unknown>) => CommandResult<unknown> | Promise<CommandResult<unknown>> | undefined

export const DEV_HOST_KEY = '__bbV2DevHost'

export function installHost(answer: Answer): void {
  const w = window as unknown as Record<string, unknown>
  w[DEV_HOST_KEY] = true
  w.__TAURI_INTERNALS__ = {
    invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
      const hit = answer(cmd, args)
      if (!hit) return Promise.reject(new Error(`${cmd}: not implemented in a browser`))
      return Promise.resolve(hit).then((r) => (r.ok ? r.value : Promise.reject(new Error(r.unsupported ? `${r.reason}: not implemented in a browser` : r.reason))))
    },
  }
}
