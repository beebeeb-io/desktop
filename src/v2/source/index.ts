import type { DriveSource } from '@beebeeb/shared/drive-ui'
import { createDesktopSource } from './desktop'
import { scenarioFromSearch } from './fixtures'
import { createBoardSource } from './fixtures/board'

/** True inside the Tauri webview. A plain browser (headless screenshots, `bun run dev`) has no IPC bridge. */
export function hasTauri(): boolean {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return false
  // Dev server only: the v2 fixture host (./dev/host.ts) stands in for Rust behind `invoke` in a plain browser; it is not Tauri.
  return !(import.meta.env.DEV && '__bbV2DevHost' in window)
}

/**
 * `?source=fixtures` always gets the board's fixtures (./fixtures/board.ts, the one set the frames render too); a plain
 * browser gets them too, so every frame renders headless.
 * Inside Tauri without the parameter: the desktop source.
 */
export function pickSource(search: string = window.location.search): DriveSource {
  const params = new URLSearchParams(search)
  if (params.get('source') === 'fixtures' || !hasTauri()) return createBoardSource(scenarioFromSearch(search))
  return createDesktopSource()
}
