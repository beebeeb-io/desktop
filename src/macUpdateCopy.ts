/**
 * What a Mac says and does about an update (task 2176, lead decision 2026-10-24 22:47; two-surface spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §10.2 #28 and E27's amendment).
 *
 * The App Sandbox denies the in-app install, so on macOS an update is downloaded, never installed in place: there is no
 * "Restart to update" and no command that would try one. Every surface that offers an update (Settings › About, the
 * popover's update row) takes its words and its address from here, so they cannot drift apart.
 */

/** The newest Mac disk image: the site redirects to the release's `.dmg`. */
export const MACOS_DOWNLOAD_URL = 'https://beebeeb.io/download/macos-dmg'

export const UPDATE_DOWNLOAD_LABEL = 'Download update'

/** The one sentence that says what to do with the download. */
export const UPDATE_DOWNLOAD_HINT = 'Drag Beebeeb into Applications to replace this version.'

export function updateAvailableLine(version: string): string {
  return `Version ${version} is available.`
}
