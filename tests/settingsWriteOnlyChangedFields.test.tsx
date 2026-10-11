/**
 * Task 1889, fix round 1 (M5): a settings write carries only the fields that changed.
 *
 * The Settings windows used to send back the whole config object they read when they opened, so changing a
 * speed after the tray paused sync wrote the old `pause_sync: false` over the pause. `set_desktop_config`
 * now merges only the fields it is sent (Rust: `config::tests::test_1889_m5_*`, `lib.rs`
 * `test_1889_m5_a_speed_changed_after_a_tray_pause_leaves_the_pause`), and these tests pin the senders:
 * the pages `Bandwidth` and `Notifications`, run with the component harness against a disk that merges
 * what it is sent, as the real command does; `MacSettings`' hook is pinned in `macSettingsTabs.test.tsx`; and
 * a source census over `src` finds no `set_desktop_config` call that sends a whole object.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { invoke } from '@tauri-apps/api/core'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { loadComponent, mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

const wholeConfig = {
  upload_kbps_limit: 0,
  download_kbps_limit: 0,
  pause_sync: false,
  notify_conflicts: true,
  notify_sync_complete: false,
  notify_quota_warnings: true,
}

/** A disk that does what `set_desktop_config` does: merges only the fields it is sent. */
function disk() {
  const state = { config: structuredClone(wholeConfig) as Record<string, unknown>, writes: [] as any[] }
  const backend = {
    get_desktop_config: () => structuredClone(state.config),
    set_desktop_config: ({ config }: any) => {
      state.writes.push(structuredClone(config))
      state.config = { ...state.config, ...structuredClone(config) }
    },
  }
  return { state, backend }
}

describe('Bandwidth page', () => {
  const formatLimit = loadComponent('pages/Bandwidth.tsx', 'formatLimit', {})
  // The page debounces a slider by 500 ms; here the timer fires on the next microtask, so `flush` settles the write.
  const bindings = {
    invoke,
    formatLimit,
    DEFAULT_CONFIG: wholeConfig,
    SLIDER_MAX_KBPS: 100_000,
    DEBOUNCE_MS: 500,
    setTimeout: (fn: () => void) => {
      queueMicrotask(fn)
      return 0
    },
    clearTimeout() {},
  }

  test('a speed changed after a pause from the tray writes only that speed, and the pause stays', async () => {
    const { state, backend } = disk()
    const m = mount('pages/Bandwidth.tsx', 'Bandwidth', { backend, bindings })
    mounted.push(m)
    await m.flush()
    state.config.pause_sync = true // the tray pauses sync while this page is open
    const upload = m.elements().find((el) => el.type === 'input' && el.props.type === 'range')!
    upload.props.onChange({ target: { value: '2000' } })
    await m.flush()
    expect(state.writes).toEqual([{ upload_kbps_limit: 2000 }])
    expect(state.config.pause_sync).toBe(true)
    expect(state.config.upload_kbps_limit).toBe(2000)
  })

  test('the pause switch writes only the pause', async () => {
    const { state, backend } = disk()
    const m = mount('pages/Bandwidth.tsx', 'Bandwidth', { backend, bindings })
    mounted.push(m)
    await m.flush()
    state.config.download_kbps_limit = 7000 // another window changed a speed meanwhile
    const pause = m.elements().find((el) => el.type === 'input' && el.props.type === 'checkbox')!
    pause.props.onChange({ target: { checked: true } })
    await m.flush()
    expect(state.writes).toEqual([{ pause_sync: true }])
    expect(state.config.download_kbps_limit).toBe(7000)
  })

  test('a write that failed goes out again with the next one, under the newer value', async () => {
    const { state, backend } = disk()
    let failNext = true
    const m = mount('pages/Bandwidth.tsx', 'Bandwidth', {
      backend: {
        ...backend,
        set_desktop_config: (args: any) => {
          if (failNext) {
            failNext = false
            throw new Error('disk full')
          }
          return backend.set_desktop_config(args)
        },
      },
      bindings,
    })
    mounted.push(m)
    await m.flush()
    const [upload, download] = m.elements().filter((el) => el.type === 'input' && el.props.type === 'range')
    upload.props.onChange({ target: { value: '2000' } })
    await m.flush()
    expect(state.writes).toEqual([])
    download.props.onChange({ target: { value: '3000' } })
    await m.flush()
    expect(state.writes).toEqual([{ upload_kbps_limit: 2000, download_kbps_limit: 3000 }])
  })
})

// Fix round 2 (N1): the retry of a failed write must never put an older value over a newer one that already went out.
describe('Bandwidth page, a failed write and a newer one', () => {
  const formatLimit = loadComponent('pages/Bandwidth.tsx', 'formatLimit', {})
  const bindings = {
    invoke,
    formatLimit,
    DEFAULT_CONFIG: wholeConfig,
    SLIDER_MAX_KBPS: 100_000,
    DEBOUNCE_MS: 500,
    setTimeout: (fn: () => void) => {
      queueMicrotask(fn)
      return 0
    },
    clearTimeout() {},
  }

  /** The first `count` writes stay in flight until the test rejects them; later ones land on the disk. */
  function pendingWrites(count: number) {
    const { state, backend } = disk()
    const rejects: Array<(error: Error) => void> = []
    const resolves: Array<() => void> = []
    let calls = 0
    const m = mount('pages/Bandwidth.tsx', 'Bandwidth', {
      backend: {
        ...backend,
        set_desktop_config: (args: any) => {
          calls += 1
          if (calls <= count) {
            state.writes.push(structuredClone(args.config)) // it went out
            return new Promise<void>((resolve, reject) => {
              rejects.push(reject)
              // It lands on the disk when the test lets it.
              resolves.push(() => {
                state.config = { ...state.config, ...structuredClone(args.config) }
                resolve()
              })
            })
          }
          return backend.set_desktop_config(args)
        },
      },
      bindings,
    })
    mounted.push(m)
    return { state, m, rejects, resolves }
  }

  test('an older write that fails after a newer one landed does not come back over it', async () => {
    const { state, m, rejects } = pendingWrites(1)
    await m.flush()
    const pause = m.elements().find((el) => el.type === 'input' && el.props.type === 'checkbox')!
    const upload = m.elements().find((el) => el.type === 'input' && el.props.type === 'range')!
    pause.props.onChange({ target: { checked: true } }) // write A: pause, in flight
    pause.props.onChange({ target: { checked: false } }) // write B: resume, goes out and lands
    await m.flush()
    expect(state.writes).toEqual([{ pause_sync: true }, { pause_sync: false }])
    rejects[0](new Error('disk full')) // A fails after B landed
    await m.flush()
    upload.props.onChange({ target: { value: '2000' } }) // the next write
    await m.flush()
    expect(state.writes.at(-1)).toEqual({ upload_kbps_limit: 2000 })
    expect(state.config.pause_sync).toBe(false)
  })

  // PR #124's review thread, exactly: two saves overlap (Pause toggled twice), the older request fails after the newer one
  // already captured and cleared the unsent patch, and a later slider change must not resend the older value over the
  // newer successful choice. Here the newer save is still in flight when the older one fails and the slider moves.
  test('the review thread: toggle Pause twice, the older request fails while the newer is in flight, then a slider moves', async () => {
    const { state, m, rejects, resolves } = pendingWrites(2)
    await m.flush()
    const pause = m.elements().find((el) => el.type === 'input' && el.props.type === 'checkbox')!
    const upload = m.elements().find((el) => el.type === 'input' && el.props.type === 'range')!
    pause.props.onChange({ target: { checked: true } }) // the older save: pause
    pause.props.onChange({ target: { checked: false } }) // the newer save: resume; both are in flight now
    await m.flush()
    expect(state.writes).toEqual([{ pause_sync: true }, { pause_sync: false }])
    rejects[0](new Error('disk full')) // the older request fails; the newer has already cleared the unsent patch
    await m.flush()
    upload.props.onChange({ target: { value: '2000' } }) // a later slider change, while the newer save is still out
    await m.flush()
    expect(state.writes.at(-1)).toEqual({ upload_kbps_limit: 2000 })
    resolves[1]() // the newer save lands
    await m.flush()
    expect(state.config.pause_sync).toBe(false)
    expect(state.writes).toEqual([{ pause_sync: true }, { pause_sync: false }, { upload_kbps_limit: 2000 }])
  })

  test('when both fail, the newer value is the one that is retried, whatever order they fail in', async () => {
    for (const order of [[0, 1], [1, 0]]) {
      const { state, m, rejects } = pendingWrites(2)
      await m.flush()
      const pause = m.elements().find((el) => el.type === 'input' && el.props.type === 'checkbox')!
      const upload = m.elements().find((el) => el.type === 'input' && el.props.type === 'range')!
      pause.props.onChange({ target: { checked: true } })
      pause.props.onChange({ target: { checked: false } })
      await m.flush()
      for (const at of order) rejects[at](new Error('disk full'))
      await m.flush()
      upload.props.onChange({ target: { value: '2000' } })
      await m.flush()
      expect(state.writes.at(-1)).toEqual({ pause_sync: false, upload_kbps_limit: 2000 })
      expect(state.config.pause_sync).toBe(false)
    }
  })
})

describe('Notifications page', () => {
  test('flipping one switch writes only that switch, and a pause made elsewhere stays', async () => {
    const { state, backend } = disk()
    const m = mount('pages/Notifications.tsx', 'Notifications', {
      backend,
      bindings: { invoke, DEFAULT_CONFIG: wholeConfig, TOGGLES: [{ key: 'notify_conflicts', label: 'Conflict alerts', description: '' }] },
    })
    mounted.push(m)
    await m.flush()
    state.config.pause_sync = true
    const toggle = m.elements().find((el) => el.type === 'input' && el.props.type === 'checkbox')!
    toggle.props.onChange()
    await m.flush()
    expect(state.writes).toEqual([{ notify_conflicts: false }])
    expect(state.config.pause_sync).toBe(true)
  })
})

describe('every sender of set_desktop_config', () => {
  function sources(dir: string): string[] {
    return readdirSync(dir).flatMap((name) => {
      const path = join(dir, name)
      if (statSync(path).isDirectory()) return sources(path)
      return /\.(ts|tsx)$/.test(name) ? [path] : []
    })
  }

  test('none sends a whole config: each sends a patch', () => {
    const found: Array<{ file: string; sent: string }> = []
    for (const file of sources(join(import.meta.dir, '..', 'src'))) {
      const text = readFileSync(file, 'utf8')
      for (const match of text.matchAll(/'set_desktop_config'\s*,\s*\{\s*config:\s*([^}]+?)\s*\}/g)) {
        found.push({ file: file.slice(file.indexOf('/src/') + 1), sent: match[1] })
      }
    }
    // The four senders: the Mac Settings hook, the Windows/Linux Settings view, and the two pages.
    expect(found.map((f) => f.file).sort()).toEqual([
      'src/MacSettings.tsx',
      'src/pages/Bandwidth.tsx',
      'src/pages/Notifications.tsx',
      'src/windows/views/SettingsView.tsx',
    ])
    for (const { file, sent } of found) {
      expect(['entry.patch', 'body', 'patch']).toContain(sent)
      expect(sent, `${file} sends a whole object`).not.toMatch(/next|latest|config\b/)
    }
  })
})
