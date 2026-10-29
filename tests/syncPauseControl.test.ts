// Settings › Sync's Pause / Resume (0.8.15) is the app menu's pause, not a second one: the same backend, the same
// failure sentences. Source checks against lib.rs, because the menu's half lives in Rust.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { PAUSE_FAILED, RESUME_FAILED } from '../src/windows/syncActivityUi'

const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
const api = readFileSync(new URL('../src/desktopApi.ts', import.meta.url), 'utf8')

function rustConst(name: string): string {
  const match = lib.match(new RegExp(`const ${name}: &str = "([^"]*)";`))
  if (!match) throw new Error(`lib.rs has no ${name}`)
  return match[1]
}

function rustFn(name: string): string {
  const start = lib.indexOf(`async fn ${name}(`)
  if (start < 0) throw new Error(`lib.rs has no ${name}`)
  return lib.slice(start, lib.indexOf('\n}\n', start))
}

describe('Settings › Sync pause control', () => {
  test('a failed Pause / Resume says what the menu says', () => {
    expect(PAUSE_FAILED.message).toBe(rustConst('MENU_PAUSE_FAILED_SENTENCE'))
    expect(RESUME_FAILED.message).toBe(rustConst('MENU_RESUME_FAILED_SENTENCE'))
  })

  test('it calls the commands that write the same flag the menu writes', () => {
    expect(api).toMatch(/command<void>\(paused \? 'tray_pause_sync' : 'tray_resume_sync'\)/)
    expect(rustFn('tray_pause_sync')).toContain('set_sync_paused(&app, true)')
    expect(rustFn('tray_resume_sync')).toContain('set_sync_paused(&app, false)')
    expect(rustFn('toggle_sync_paused_from_menu')).toContain('set_sync_paused(app, pausing)')
    // `set_sync_paused` announces the change on `engine-status`, which every open surface listens to.
    expect(rustFn('set_sync_paused')).toContain('app.emit("engine-status"')
  })

  test('both commands and the summary read are registered', () => {
    const start = lib.indexOf('tauri::generate_handler![')
    const handlers = lib.slice(start, lib.indexOf('])', start))
    for (const name of ['tray_pause_sync', 'tray_resume_sync', 'sync_activity_summary']) {
      expect(handlers).toMatch(new RegExp(`\\b${name},`))
    }
  })
})
