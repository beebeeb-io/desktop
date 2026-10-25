/**
 * What a Mac says about an available update (task 2176, lead decision 2026-10-24 under Guus's standing "take
 * decisions"). The App Sandbox denies the in-app install (tauri-plugin-updater's bundle swap), so on macOS an update
 * is downloaded, never installed in place: "Download update" opens the newest Mac disk image in the browser, with one
 * sentence that says what to do with it. The popover's update row (task 1683 slice 3) uses these; Settings › About
 * (slice 5) is meant to use the same ones.
 */

/** A redirect to the newest Mac disk image (verified for 0.8.12 on 2026-10-24). */
export const MACOS_DOWNLOAD_URL = 'https://beebeeb.io/download/macos-dmg'
export const UPDATE_DOWNLOAD_LABEL = 'Download update'
export const UPDATE_DOWNLOAD_HINT = 'Drag Beebeeb into Applications to replace this version.'

/** "Version 0.8.14 is available." (the shipped words of `UpdateBanner` and `updateRow`). */
export function updateAvailableLine(version: string): string {
  return `Version ${version} is available.`
}
