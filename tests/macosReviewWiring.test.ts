import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const host = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')

test('native Review and Trash requests reach the single main window', () => {
  expect(host).toContain('Some("review") => MainNav::Review')
  expect(host).toContain('Some("trash") => MainNav::Trash')
  for (const [name, digit] of [['Review', 2], ['Trash', 3]]) {
    expect(host).toContain(`MainMenuNav::${name} => show_main_window(app, MainNav::${name})`)
    expect(host).toContain(`accelerator: Some("CmdOrCtrl+Digit${digit}")`)
  }
})

test('a Mac conflict request never builds a third webview', () => {
  const body = host.slice(host.indexOf('pub(crate) fn open_conflict_window_impl('), host.indexOf('async fn resolve_conflict('))
  const mac = body.slice(body.indexOf('#[cfg(target_os = "macos")]'), body.indexOf('#[cfg(not(target_os = "macos"))]'))
  expect(mac).toContain('show_main_window(app, MainNav::Review)')
  expect(mac).not.toContain('WebviewWindowBuilder')
})

test('Review count and list commands run away from the UI thread', () => {
  expect(host).toContain('async fn review_item_count(')
  expect(host).toContain('async fn list_version_conflict_center(')
  expect(host.match(/^            review_item_count,$/gm)).toHaveLength(1)
  expect(host).toContain('on_the_blocking_pool("Could not read Review."')
})
