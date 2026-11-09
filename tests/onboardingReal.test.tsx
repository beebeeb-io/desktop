/**
 * First run sets up a real folder (36-onboarding-real).
 *
 * The Windows window used to walk straight into the passkey step, so `pick_sync_root` was never called and the app
 * could finish setup with no Beebeeb folder. These tests drive `SetupFlow`'s real handlers with the step views stubbed,
 * so each step's action is asserted where the controller decides it:
 * - no folder: choosing one runs the picker, and the flow only advances after the command returned a real path;
 * - an existing folder (already in the config): shown, and Continue moves on without opening the picker again;
 * - File Explorer registration failed: Try again re-reads the state and retries the folder the config holds;
 * - Done: a missing folder is named, and the heading never claims the PC is set up without one.
 *
 * What this does NOT prove: the rendered layout, or Windows' native picker (the walkthrough covers the window).
 */
import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import * as onboarding from '../src/v2/onboarding'
import type { ShellIntegrationView } from '../src/v2/onboarding'
import { DoneStep } from '../src/v2/onboarding/views'
import { mount, type Handler } from './fixtures/componentHarness'

const WINDOWS_PATH = 'C:\\Users\\Sam\\Beebeeb'

const shell = (o: Partial<ShellIntegrationView> = {}): ShellIntegrationView => ({
  installed: false, path: WINDOWS_PATH, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null, ...o,
})

/** The commands the controller uses; every read has an honest default so a test names only what it cares about. */
function backend(overrides: Record<string, Handler> = {}) {
  const calls: Array<{ name: string; args: any }> = []
  const answers: Record<string, Handler> = {
    get_sync_root: () => null,
    autostart_enabled: () => false,
    windows_shell_integration_state: () => shell(),
    install_windows_shell_integration: () => shell({ installed: true, status: 'installed' }),
    ...overrides,
  }
  const command = async (name: string, args?: any) => {
    calls.push({ name, args })
    const answer = answers[name]
    if (!answer) return { ok: false as const, reason: 'unsupported', unsupported: true }
    try {
      return { ok: true as const, value: await answer(args) }
    } catch (error) {
      return { ok: false as const, reason: String(error) }
    }
  }
  return { command, calls }
}

function mark(name: string) {
  return Object.defineProperty(function () { return null }, 'name', { value: name }) as (props: any) => null
}

function mountFlow(command: (name: string, args?: any) => Promise<any>, step = 'folder') {
  const stubs = {
    setup: mark('SetupWindow'), folder: mark('FolderStep'), passkey: mark('PasskeyStep'), lock: mark('LockStep'),
    backup: mark('BackupStep'), photos: mark('PhotosStep'), compare: mark('CompareView'), explorer: mark('ExplorerStep'),
    notifications: mark('NotificationsStep'), login: mark('LoginStep'), done: mark('DoneStep'),
  }
  const m = mount('v2/onboarding/index.tsx', 'SetupFlow', {
    backend: {},
    bindings: {
      command,
      getKnownFolderBackup: async () => ({ ok: true, value: [] }),
      setKnownFolderBackup: async () => ({ ok: true }),
      markKnownFolderOnboardingSeen: async () => ({ ok: true }),
      startGuide: () => undefined,
      getCurrentWindow: () => ({ close: async () => undefined }),
      seed: (key: string, fallback: unknown) => (key === 'setup.step' ? step : fallback),
      DEFAULT_ON: onboarding.DEFAULT_ON,
      toBackupFolders: onboarding.toBackupFolders,
      explorerStatus: onboarding.explorerStatus,
      needsFirstRegistration: onboarding.needsFirstRegistration,
      shellPolls: onboarding.shellPolls,
      isRealFolder: onboarding.isRealFolder,
      SAMPLE: { label: '4K HEIC', size: '1.0 MB', original: '30.9 MB', photos: 0, local: '0 GB', cloud: '0 GB', space: '20 GB' },
      SetupWindow: stubs.setup,
      FolderStep: stubs.folder,
      PasskeyStep: stubs.passkey,
      LockStep: stubs.lock,
      BackupStep: stubs.backup,
      PhotosStep: stubs.photos,
      CompareView: stubs.compare,
      ExplorerStep: stubs.explorer,
      NotificationsStep: stubs.notifications,
      LoginStep: stubs.login,
      DoneStep: stubs.done,
    },
  })
  /** The stub a step rendered this time, with the props the controller gave it. */
  const find = (name: keyof typeof stubs) => m.elements().find((el) => el.type === stubs[name])
  return { m, find, close: () => m.close() }
}

test('no folder: the picker saves a real path, and the flow advances only after it returned', async () => {
  let resolvePick: (value: string) => void = () => undefined
  const { command, calls } = backend({ pick_sync_root: () => new Promise<string>((resolve) => { resolvePick = resolve }) })
  const { m, find, close } = mountFlow(command)
  try {
    await m.flush()
    expect(find('setup')!.props.step).toBe('folder')
    expect(find('folder')!.props.path).toBe(null)

    find('folder')!.props.onChoose()
    await m.flush()
    // The picker is open: the step has not advanced and no path is claimed yet.
    expect(calls.map((c) => c.name)).toContain('pick_sync_root')
    expect(find('folder')!.props.path).toBe(null)
    expect(find('setup')!.props.step).toBe('folder')

    resolvePick(WINDOWS_PATH)
    await m.flush()
    await m.flush()
    expect(find('setup')!.props.step).toBe('passkey')
    expect(calls.filter((c) => c.name === 'pick_sync_root')).toHaveLength(1)
  } finally { close() }
})

test('an existing folder is shown, and Continue moves on without opening the picker again', async () => {
  const { command, calls } = backend({ get_sync_root: () => WINDOWS_PATH })
  const { m, find, close } = mountFlow(command)
  try {
    await m.flush()
    expect(find('setup')!.props.step).toBe('folder')
    expect(find('folder')!.props.path).toBe(WINDOWS_PATH)

    find('folder')!.props.onContinue()
    await m.flush()
    expect(find('setup')!.props.step).toBe('passkey')
    expect(calls.map((c) => c.name)).not.toContain('pick_sync_root')
  } finally { close() }
})

test('a failed File Explorer registration offers Try again, which retries the configured folder', async () => {
  const { command, calls } = backend({
    get_sync_root: () => WINDOWS_PATH,
    windows_shell_integration_state: () => shell({ status: 'error', last_error: 'register failed (0x8007017C)' }),
  })
  const { m, find, close } = mountFlow(command, 'explorer')
  try {
    await m.flush()
    expect(find('explorer')!.props.status).toBe('failed')
    expect(find('explorer')!.props.errorCode).toBe('0x8007017C')

    find('explorer')!.props.onTryAgain()
    await m.flush()
    await m.flush()
    const install = calls.filter((c) => c.name === 'install_windows_shell_integration')
    expect(install).toHaveLength(1)
    expect(install[0].args).toEqual({ path: WINDOWS_PATH })
    expect(calls.map((c) => c.name).filter((n) => n === 'windows_shell_integration_state').length).toBeGreaterThan(1)
  } finally { close() }
})

test('Done names the missing folder and never claims set-up without one', () => {
  const summary = { passkey: 'Not now', lock: 'Off', backup: 'Off', previews: '4K HEIC', explorer: 'Adding', notifications: 'On', login: 'On' }
  const noop = () => undefined
  const missing = renderToStaticMarkup(<DoneStep summary={summary} folder={null} onRow={noop} onOpen={noop} onChooseFolder={noop} />)
  expect(missing).toContain('One thing left')
  expect(missing).toContain('A folder for your Beebeeb files')
  expect(missing).not.toContain('You’re set up')
  expect(missing).toContain('Choose folder')

  const ready = renderToStaticMarkup(<DoneStep summary={summary} folder={WINDOWS_PATH} onRow={noop} onOpen={noop} onChooseFolder={noop} />)
  expect(ready).toContain('You’re set up')
  expect(ready).toContain(WINDOWS_PATH)
  expect(ready).not.toContain('A folder for your Beebeeb files')
})
