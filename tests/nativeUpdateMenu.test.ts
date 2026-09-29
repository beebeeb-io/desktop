import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

// The native boundary cannot execute in Bun. Pin its dispatch contract alongside
// the controller's behavioral tests so a backend-only menu check cannot return.
test('native menu queues a frontend check instead of swallowing the IPC failure in Rust', () => {
  const rust = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
  const arm = rust.split('DesktopMenuAction::CheckForUpdates => {')[1].split('DesktopMenuAction::ToggleSync =>')[0]
  expect(arm).toContain('request_menu_update_check(app)')
  expect(arm).not.toContain('check_for_updates_now(')
})

test('Settings and the always-mounted update surface use the same check controller', () => {
  const settings = readFileSync(new URL('../src/windows/views/SettingsView.tsx', import.meta.url), 'utf8')
  const banner = readFileSync(new URL('../src/UpdateBanner.tsx', import.meta.url), 'utf8')
  expect(settings).toContain('desktopUpdateCheck.check')
  expect(settings).not.toContain('await checkForDesktopUpdatesNow()')
  expect(banner).toContain('<ManualUpdateFeedback />')
})
