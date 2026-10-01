/**
 * The popover's command contract (task 1683 slice 3).
 *
 * Seven of the popover's actions use commands that exist in `generate_handler!`; three do not
 * exist yet (`PENDING_COMMANDS`, owed by slice 6). This file keeps that record honest in both
 * directions, by reading `src-tauri/src/lib.rs`:
 *   - every command the popover calls through `WIRED_COMMANDS` IS registered;
 *   - every name in `PENDING_BACKEND_COMMANDS` is NOT registered (so when slice 6 adds one, this
 *     test goes red and the list is shortened in the same change, instead of rotting);
 *   - every string literal the controller passes to `call(` is one of those two sets, so a new
 *     command cannot be called from the popover without being classified.
 *
 * Mutation checks (red first, then reverted) are recorded in the task Notes, 2026-10-01.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { PENDING_BACKEND_COMMANDS, PENDING_COMMANDS, WIRED_COMMANDS } from '../src/macPopover/commands'

const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8').replace(/\r\n/g, '\n')

function registered(): Set<string> {
  const start = lib.indexOf('.invoke_handler(tauri::generate_handler![')
  expect(start).toBeGreaterThan(-1)
  // The list ends at the line that is just `])` (an attribute inside it would end a plain `]` search early).
  const end = lib.indexOf('\n        ])', start)
  expect(end).toBeGreaterThan(start)
  const body = lib.slice(start, end)
  const names = body
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, '').trim().replace(/,$/, ''))
    .filter((line) => /^[a-z_][a-z0-9_:]*$/.test(line))
    .map((line) => line.split('::').pop() as string)
  return new Set(names)
}

describe('the popover’s backend commands', () => {
  test('the handler list was read (the instrument): it knows commands we know exist', () => {
    const names = registered()
    expect(names.size).toBeGreaterThan(80) // 92 when written
    for (const known of ['popover_snapshot', 'unlock_vault', 'install_finder_location', 'tray_pause_sync']) expect(names.has(known), known).toBe(true)
    expect(names.has('definitely_not_a_command')).toBe(false)
  })

  test('every wired command is registered (6 of 6; the window hide is a plugin call)', () => {
    const names = registered()
    const wired = Object.values(WIRED_COMMANDS).filter((n) => !n.startsWith('plugin:'))
    expect(wired.length).toBe(6)
    for (const name of wired) expect(names.has(name), name).toBe(true)
  })

  test('every pending command is NOT registered yet (3 of 3): delete it from the list when slice 6 adds it', () => {
    const names = registered()
    expect(PENDING_BACKEND_COMMANDS.length).toBe(3)
    expect([...PENDING_BACKEND_COMMANDS]).toEqual(Object.values(PENDING_COMMANDS))
    for (const name of PENDING_BACKEND_COMMANDS) expect(names.has(name), `${name} is registered: remove it from PENDING_COMMANDS`).toBe(false)
  })

  test('the controller calls no command that is not classified as wired or pending', () => {
    const source = readFileSync(new URL('../src/macPopover/controller.ts', import.meta.url), 'utf8')
    const known = new Set<string>([...Object.values(WIRED_COMMANDS), ...Object.values(PENDING_COMMANDS)])
    const called = [...source.matchAll(/call\(\s*(WIRED_COMMANDS|PENDING_COMMANDS)\.(\w+)/g)].map((m) => (m[1] === 'WIRED_COMMANDS' ? WIRED_COMMANDS : PENDING_COMMANDS)[m[2] as never] as string)
    expect(called.length).toBeGreaterThanOrEqual(9)
    for (const name of called) expect(known.has(name), name).toBe(true)
    // and no literal command name is passed to call() at all
    expect([...source.matchAll(/call\(\s*['"`]/g)].length).toBe(0)
  })

  test('the contract names are the ones slice 6 was told (a rename here is a rename in Rust)', () => {
    expect(PENDING_COMMANDS).toEqual({ gearMenu: 'popover_gear_menu', unlockWithPassword: 'popover_unlock_vault', retrySync: 'popover_retry_sync' })
  })
})
