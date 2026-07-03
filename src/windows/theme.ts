import { command, DEFAULT_CONFIG, type DesktopConfig } from '../desktopApi'
import { normalizeThemePreference, resolveThemePreference, type ThemePreference } from './advancedSettingsModel'

const COLOR_SCHEME_QUERY = '(prefers-color-scheme: dark)'

let cleanup: (() => void) | null = null

function setResolvedTheme(preference: ThemePreference, systemPrefersDark: boolean, root: HTMLElement) {
  const resolved = resolveThemePreference(preference, systemPrefersDark)
  root.dataset.themePreference = preference
  root.dataset.theme = resolved
  root.style.colorScheme = resolved
}

export function applyDesktopThemePreference(
  rawPreference: unknown,
  targetDocument: Document = document,
  targetWindow: Window = window,
): () => void {
  const preference = normalizeThemePreference(rawPreference)
  const root = targetDocument.documentElement
  const media = targetWindow.matchMedia?.(COLOR_SCHEME_QUERY)
  const update = () => setResolvedTheme(preference, media?.matches ?? false, root)

  update()
  if (preference !== 'system' || !media) return () => {}

  media.addEventListener?.('change', update)
  return () => media.removeEventListener?.('change', update)
}

export function setDesktopThemePreference(preference: unknown) {
  cleanup?.()
  cleanup = applyDesktopThemePreference(preference)
}

export async function initializeDesktopThemeFromConfig() {
  setDesktopThemePreference(DEFAULT_CONFIG.theme)
  const result = await command<DesktopConfig | null>('desktop_config')
  if (result.ok) {
    setDesktopThemePreference(result.value?.theme ?? DEFAULT_CONFIG.theme)
  }
}
