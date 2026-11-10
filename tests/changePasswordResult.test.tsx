// Task 2439 item 2 (DW-A67): the server has already committed when `account_change_password` answers,
// so a failure to store the new sign-in or to restart the engine is said under "Password changed",
// never as a failed change. The sentence selection is pure (`passwordWords.ts`); this drives the real
// `ChangePassword` handlers with a scripted `run`.
import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'
import {
  passwordChangeWords,
  PASSWORD_CHANGED,
  RESTART_FAILED,
  SIGN_IN_AGAIN,
} from '../src/v2/sections/settings/account/passwordWords'

const Stub = (p: { children?: unknown }) => createElement('div', null, p.children as never)
const Btn = (p: { children?: unknown; onClick?: () => void; disabled?: boolean }) =>
  createElement('button', { onClick: p.onClick, disabled: p.disabled }, p.children as never)
const Field = (p: { children?: unknown; error?: string | null }) =>
  createElement('div', null, p.error ? createElement('p', null, p.error) : null, p.children as never)
const PasswordInput = (p: { label: string; value: string; onChange: (v: string) => void }) =>
  createElement('input', {
    'aria-label': p.label,
    value: p.value,
    onChange: (e: { target: { value: string } }) => p.onChange(e.target.value),
  })
const BackLink = (p: { label: string; onBack: () => void }) => createElement('button', { onClick: p.onBack }, p.label)
const Page = (p: { children?: unknown; back?: unknown }) => createElement('div', null, p.back as never, p.children as never)
const strength = (pw: string) => ({ label: pw.length >= 12 ? 'Strong' : '', ok: pw.length >= 12 })
const useEscBack = () => undefined
const errorText = (code: string) => code
const NEW_PASSWORD_ERRORS = ['weak_password', 'same_password', 'password_breached', 'breach_check_blocked', 'breach_check_missing']

const bindings = { Page, BackLink, Field, PasswordInput, Btn, strength, useEscBack, errorText, NEW_PASSWORD_ERRORS, passwordChangeWords }

function type(m: Mounted, label: string, value: string) {
  const input = m.elements().find((e) => e.type === 'input' && e.props['aria-label'] === label)
  if (!input) throw new Error(`no input "${label}"`)
  input.props.onChange({ target: { value } })
  m.render()
}

async function change(answer: unknown) {
  const done: unknown[] = []
  const m = mount('v2/sections/settings/account/SecurityPane.tsx', 'ChangePassword', {
    backend: {},
    bindings: { ...bindings, run: async () => answer },
    props: { onBack: () => {}, onDone: (w: unknown) => done.push(w) },
    expand: true,
  })
  type(m, 'Current password', 'old-password-1')
  type(m, 'New password', 'river-lantern-quiet-74')
  type(m, 'New password again', 'river-lantern-quiet-74')
  await m.click('Change password')
  return { m, done }
}

describe('a changed password reports what followed (DW-A67, task 2439 item 2)', () => {
  test('the server committed and the engine restarted: "Password changed", back to the security page', async () => {
    const { m, done } = await change({ ok: true, value: { status: 'changed', signedIn: true, restart: 'ok' } })
    const text = textOf(m.elements())
    expect(text).toContain(PASSWORD_CHANGED)
    expect(text).not.toContain(RESTART_FAILED)
    expect(text).not.toContain(SIGN_IN_AGAIN)
    expect(done).toEqual([{ headline: PASSWORD_CHANGED, detail: null }])
    m.close()
  })

  test('the server committed but the restart failed: "Password changed" plus the restart line', async () => {
    const { m, done } = await change({ ok: true, value: { status: 'changed', signedIn: true, restart: 'failed' } })
    const text = textOf(m.elements())
    expect(text).toContain(PASSWORD_CHANGED)
    expect(text).toContain(RESTART_FAILED)
    expect(text).not.toContain(SIGN_IN_AGAIN)
    expect(done).toEqual([{ headline: PASSWORD_CHANGED, detail: RESTART_FAILED }])
    m.close()
  })

  test('the server committed but the new sign-in could not be stored: "Password changed" plus "Sign in again on this PC."', async () => {
    const { m, done } = await change({ ok: true, value: { status: 'changed', signedIn: false, restart: 'failed' } })
    const text = textOf(m.elements())
    expect(text).toContain(PASSWORD_CHANGED)
    expect(text).toContain(SIGN_IN_AGAIN)
    expect(text).not.toContain(RESTART_FAILED)
    expect(done).toEqual([{ headline: PASSWORD_CHANGED, detail: SIGN_IN_AGAIN }])
    m.close()
  })

  test('a wrong current password still shows the existing error and never "Password changed"', async () => {
    const { m, done } = await change({ ok: false, code: 'wrong_password' })
    const text = textOf(m.elements())
    expect(text).toContain('Current password is incorrect. Nothing was changed.')
    expect(text).not.toContain(PASSWORD_CHANGED)
    expect(done).toEqual([])
    m.close()
  })
})
