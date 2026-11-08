import type { DriveSource } from '@beebeeb/shared/drive-ui'
import { createDesktopSource } from './desktop'
import { createFixtureSource, scenarioFromSearch } from './fixtures'

/** True inside the Tauri webview. A plain browser (headless screenshots, `bun run dev`) has no IPC bridge. */
export function hasTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/**
 * `?source=fixtures` always gets the fixtures; a plain browser gets them too, so every frame renders headless.
 * Inside Tauri without the parameter: the desktop source.
 */
export function pickSource(search: string = window.location.search): DriveSource {
  const params = new URLSearchParams(search)
  if (params.get('source') === 'fixtures' || !hasTauri()) return createFixtureSource(scenarioFromSearch(search))
  return createDesktopSource()
}
