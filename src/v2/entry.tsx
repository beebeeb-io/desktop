// Desktop v2's entry, loaded on demand by src/main.tsx so its CSS (Tailwind, the board's tokens) never
// reaches a window that is still on the legacy stylesheet.
import './v2.css'

/** The desktop theme sets data-theme on <html>; the shared tokens switch on `.dark`. `?theme=` overrides it for screenshots. */
export function followTheme(root: HTMLElement = document.documentElement): void {
  root.classList.add('v2')
  const forced = new URLSearchParams(window.location.search).get('theme')
  const apply = () => root.classList.toggle('dark', forced ? forced === 'dark' : root.dataset.theme === 'dark')
  apply()
  if (!forced) new MutationObserver(apply).observe(root, { attributes: true, attributeFilter: ['data-theme'] })
}

export { default as V2App } from './V2App'
export { default as TrayApp } from './tray'
export { default as OnboardingApp } from './onboarding'
