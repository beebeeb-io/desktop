import { describe, expect, test } from 'bun:test'
import { readFileSync, existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { POPOVER_HIDDEN_EVENT, POPOVER_SHOWN_EVENT } from '../src/popoverContract'

const root = fileURLToPath(new URL('..', import.meta.url))
const read = (path: string) => existsSync(`${root}/${path}`) ? readFileSync(`${root}/${path}`, 'utf8') : ''
const config = JSON.parse(read('src-tauri/tauri.macos.conf.json'))
const native = () => read('src-tauri/src/macos_popover.rs')

describe('the shipping macOS popover boundary', () => {
  test('only two macOS windows, both created on demand, with a fixed opaque popover', () => {
    expect(config.app.windows.map((w: { label: string }) => w.label).sort()).toEqual(['main', 'popover'])
    const pop = config.app.windows.find((w: { label: string }) => w.label === 'popover')
    expect(pop).toMatchObject({ create: false, visible: false, width: 372, height: 488, resizable: false, decorations: false, transparent: false, alwaysOnTop: true, skipTaskbar: true, url: 'index.html?window=popover&platform=macos' })
    expect(config.app.trayIcon.showMenuOnLeftClick).toBe(false)
  })

  test('the shipping entry mounts the popover without a toast host', () => {
    const entry = read('src/main.tsx')
    expect(entry).toContain("which === 'popover' && platform === 'macos'")
    expect(entry).toContain('component = <PopoverApp />')
    expect(entry).toContain("which === 'popover' ?")
    expect(entry).toContain("import './popover.css'")
  })

  test('its capability is macOS-only and grants no updater, dialog, or window-state plugin', () => {
    const cap = JSON.parse(read('src-tauri/capabilities/macos-popover.json') || '{}')
    expect(cap.platforms).toEqual(['macOS'])
    expect(cap.windows).toEqual(['popover'])
    expect(cap.permissions).toContain('opener:default')
    expect(cap.permissions.some((p: string) => /updater|dialog|window-state/.test(p))).toBe(false)
  })

  test('every native command invoked by PopoverApp is registered exactly once', () => {
    const commands = [...read('src/PopoverApp.tsx').matchAll(/command(?:<[^>]*>)?\('([^']+)'/g)].map((m) => m[1]).filter((name) => name.startsWith('popover_') && name !== 'popover_snapshot')
    expect(new Set(commands).size).toBe(6)
    const registered = read('src-tauri/src/lib.rs')
    for (const name of new Set(commands)) expect(registered.match(new RegExp(`^            macos_popover::${name},$`, 'gm')), name).toHaveLength(1)
  })

  test('tray click uses the session and blur guard; macOS installs no status-item context menu', () => {
    expect(native()).toContain('auth_health.is_expired()')
    expect(native()).toContain('on_tray_click(')
    expect(native()).toContain('record_blur_hide(')
    expect(read('src-tauri/src/lib.rs')).toContain('macos_popover::toggle(app)')
    expect(read('src-tauri/src/lib.rs')).toContain('#[cfg(not(target_os = "macos"))]\n    tray.set_menu(Some(tray_menu))?;')
  })

  test('every way the popover hides tells it, so it stops reading until it is shown again (review P2-2 of #160)', () => {
    // Esc, the blur, a blur after the gear menu, the tray click and opening the Beebeeb window all hide the window;
    // each must go through the one function that also emits the hidden event.
    const src = native()
    expect(src.match(/\.hide\(\)/g)).toHaveLength(1)
    const start = src.indexOf('fn conceal(')
    expect(start).toBeGreaterThan(0)
    const body = src.slice(start, src.indexOf('\n}\n', start))
    expect(body).toContain('.hide()')
    expect(body).toMatch(/emit\((crate::popover_data::)?POPOVER_HIDDEN_EVENT, /)
    expect(src).toMatch(/emit\((crate::popover_data::)?POPOVER_SHOWN_EVENT, /)
    const data = read('src-tauri/src/popover_data.rs')
    expect(data).toContain(`pub const POPOVER_SHOWN_EVENT: &str = "${POPOVER_SHOWN_EVENT}";`)
    expect(data).toContain(`pub const POPOVER_HIDDEN_EVENT: &str = "${POPOVER_HIDDEN_EVENT}";`)
  })

  test('the gear uses a native menu and the existing handlers, including pause and update checks', () => {
    expect(native()).toContain('popup_menu_at(')
    for (const id of ['MENU_PREFERENCES_ID', 'MENU_CHECK_UPDATES_ID', 'MENU_TOGGLE_SYNC_ID', 'MENU_QUIT_ID', 'MENU_HELP_DOCS_ID', 'MENU_HELP_SHORTCUTS_ID', 'MENU_HELP_REPORT_ID', 'MENU_HELP_STATUS_ID']) expect(native(), id).toContain(id)
    expect(native()).toContain('handle_desktop_menu_action(')
  })

  test('missing keys hands off to Set up this Mac without reporting an unlock', () => {
    expect(native()).toContain('Err(error) if error == NO_VAULT_KEY_UNLOCK_ERROR')
    expect(native()).toContain('show_main_window(&app, MainNav::Setup)')
  })

  test('opening the Beebeeb window hides the popover, including an existing window', () => {
    const lib = read('src-tauri/src/lib.rs')
    const start = lib.indexOf('fn show_main_window(')
    const end = lib.indexOf('\n}\n', start)
    const body = lib.slice(start, end)
    expect(body.indexOf('macos_popover::hide(app)')).toBeGreaterThan(0)
    expect(body.indexOf('macos_popover::hide(app)')).toBeLessThan(body.indexOf('if let Some(win)'))
  })
})
