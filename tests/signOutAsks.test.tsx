/**
 * Slice 56: sign out finishes or says why. On Windows, `clear_session` with "keep" copies every file from before the
 * last update inside the command, so the button sat on "signing out" for the whole copy and the question came after.
 * Both v2 sign-outs (Settings' `SignOutDialog` and the no-plan screen's `NoPlanWindow`) now ask first, with the legacy
 * window's words (`unknownFilesSignOutCopy`), and pass the answer to `clear_session`. Cancel calls nothing; a failed
 * `clear_session` keeps the question open with the reason and leaves its buttons usable.
 *
 * Run with the component harness against a scripted backend. The dialog is loaded from the production file when this
 * slice's code is present; before it, a stub keeps the red run on an assertion (the question is not asked) rather than
 * a missing component.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { clearSession, command, signOutUnknownFiles } from '../src/desktopApi'
import { unknownFilesSignOutCopy } from '../src/signOutUnknownFilesCopy'
import { elementsOf, loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const COUNT = 51217
const FOLDER = 'C:\\Users\\GuusLangelaar\\Beebeeb kept files 2026-11-10 0909'
const COPY = unknownFilesSignOutCopy(COUNT, FOLDER)
const REFUSED = 'Sign-out paused: the sync folder could not be read.'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const clearCalls = (m: Mounted) => m.calls.filter((c) => c.name === 'clear_session')

function dialogComponent() {
  try {
    return loadComponent('v2/sections/settings/account/SecurityPane.tsx', 'SignOutUnknownFilesDialog', {
      React: { createElement }, Dialog: 'settings-dialog', Btn: 'settings-btn', unknownFilesSignOutCopy,
    })
  } catch {
    return ({ count, folder }: { count: number; folder: string }) =>
      createElement('settings-dialog', { title: unknownFilesSignOutCopy(count, folder).title })
  }
}
const SignOutUnknownFilesDialog = dialogComponent()

function settingsDialog(backend: Record<string, (args: any) => unknown>) {
  const m = mount('v2/sections/settings/account/SecurityPane.tsx', 'SignOutDialog', {
    backend,
    bindings: { clearSession, signOutUnknownFiles, hasTauri: () => true, Dialog: 'settings-dialog', Btn: 'settings-btn', SignOutUnknownFilesDialog },
    props: { all: false, onCancel() {} },
    expand: true,
  })
  mounted.push(m)
  ;(globalThis as any).window.location = { assign() {} }
  const dialog = () => m.elements().find((el) => el.type === 'settings-dialog')!
  const buttons = () => elementsOf(dialog().props.actions).filter((el) => el.type === 'settings-btn')
  const button = (label: string) => buttons().find((el) => textOf(el.props.children) === label)!
  const confirm = async () => {
    const b = buttons().find((el) => el.props.variant === 'danger')!
    b.props.onClick()
    await m.flush()
  }
  return { m, dialog, buttons, button, confirm }
}

function noPlan(backend: Record<string, (args: any) => unknown>) {
  const signedOut: Array<string | null> = []
  const m = mount('v2/auth/Gate.tsx', 'NoPlanWindow', {
    backend,
    bindings: { clearSession, signOutUnknownFiles, command, PLANS_URL: 'https://beebeeb.io/pricing', NoPlanView: 'no-plan-view', SignOutUnknownFilesDialog },
    props: { onCheckAgain() {}, onSignedOut: (w: string | null) => signedOut.push(w) },
    expand: true,
  })
  mounted.push(m)
  const view = () => m.elements().find((el) => el.type === 'no-plan-view')!
  const signOut = async () => { view().props.onSignOut(); await m.flush() }
  const dialog = () => m.elements().find((el) => el.type === 'settings-dialog')
  const button = (label: string) => elementsOf(dialog()!.props.actions).find((el) => el.type === 'settings-btn' && textOf(el.props.children) === label)!
  return { m, view, signOut, dialog, button, signedOut }
}

describe('sign out asks about files from before the last update', () => {
  test('(a) a count above zero shows the question and clears nothing, in Settings and on the no-plan screen', async () => {
    const s = settingsDialog({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => ({ warning: null }),
    })
    await s.confirm()
    expect(s.dialog().props.title).toBe(COPY.title)
    expect(clearCalls(s.m)).toHaveLength(0)

    const n = noPlan({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => ({ warning: null }),
    })
    await n.signOut()
    expect(n.dialog()!.props.title).toBe(COPY.title)
    expect(clearCalls(n.m)).toHaveLength(0)
  })

  test('(b) Keep copies signs out with keep and the folder the question named', async () => {
    const s = settingsDialog({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => ({ warning: null }),
    })
    await s.confirm()
    s.button(COPY.keepLabel).props.onClick()
    await s.m.flush()
    expect(clearCalls(s.m).map((c) => c.args)).toEqual([{ unknownFiles: 'keep', keptFolder: FOLDER }])
  })

  test('(c) Delete them signs out with delete', async () => {
    const s = settingsDialog({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => ({ warning: null }),
    })
    await s.confirm()
    s.button(COPY.deleteLabel).props.onClick()
    await s.m.flush()
    expect(clearCalls(s.m).map((c) => c.args)).toEqual([{ unknownFiles: 'delete' }])
  })

  test('(d) a count of zero clears once with no unknownFiles', async () => {
    const s = settingsDialog({
      sign_out_unknown_files: () => ({ count: 0, folder: null }),
      clear_session: () => ({ warning: null }),
    })
    await s.confirm()
    expect(clearCalls(s.m)).toHaveLength(1)
    expect(clearCalls(s.m)[0]!.args.unknownFiles).toBeUndefined()
    expect(s.dialog().props.title).toBe('Sign out of Drive?')
  })

  test('(e) a failed clearSession keeps the question open with the reason and the button enabled, in Settings and on the no-plan screen', async () => {
    const s = settingsDialog({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => { throw new Error(REFUSED) },
    })
    await s.confirm()
    s.button(COPY.keepLabel).props.onClick()
    await s.m.flush()
    expect(s.dialog().props.title).toBe(COPY.title)
    expect(s.m.elements().filter((el) => el.props.role === 'alert').map((el) => textOf(el.props.children))).toEqual([REFUSED])
    expect(s.button(COPY.keepLabel).props.disabled).toBe(false)

    const n = noPlan({
      sign_out_unknown_files: () => ({ count: COUNT, folder: FOLDER }),
      clear_session: () => { throw new Error(REFUSED) },
    })
    await n.signOut()
    n.button(COPY.keepLabel).props.onClick()
    await n.m.flush()
    expect(n.dialog()!.props.title).toBe(COPY.title)
    expect(n.m.elements().filter((el) => el.props.role === 'alert').map((el) => textOf(el.props.children))).toEqual([REFUSED])
    expect(n.button(COPY.keepLabel).props.disabled).toBe(false)
  })
})
