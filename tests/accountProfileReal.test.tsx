// Account shows the profile the commands returned. A verified address says Verified, an unverified one offers
// Verify, a failed read shows that error and Try again, and a rename goes through account_set_name. No fixture account.
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, type ReactNode } from 'react'
import { command } from '../src/desktopApi'
import { ERRORS, errorText, initialAccountState, run } from '../src/v2/sections/settings/account/data'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

/** The real command. A third argument is the browser stand-in `run` treats as success, which is not a save. */
async function runCommand<T>(name: string, args: Record<string, unknown>, fake?: T): Promise<{ ok: true; value: T } | { ok: false; code: string }> {
  if (fake !== undefined) return { ok: false, code: 'browser_stand_in' }
  return run<T>(name, args)
}

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const TOKEN = 'tok_session_91ab'
const EMAIL = 'sam@example.eu'

function profile(email: string, emailVerified: boolean) {
  return { user_id: TOKEN, email, email_verified: emailVerified, created_at: '2026-01-01T00:00:00Z', totp_enabled: false }
}

type Read = { profile?: () => unknown; displayName?: string | null }

function reads(over: Read = {}) {
  const displayName = over.displayName === undefined ? 'Sam' : over.displayName
  return {
    account_profile: over.profile ?? (() => profile(EMAIL, true)),
    account_display_name: () => ({ display_name: displayName }),
    account_session_list: () => ({ sessions: [] }),
    account_security_score: () => ({ factors: [] }),
    account_activity: () => ({ events: [] }),
    account_activity_logging: () => ({ on: true }),
    account_passkeys_list: () => [],
    sync_status: () => ({ logged_in: true }),
    app_version: () => ({ version: '0.9.4', release: true }),
    desktop_platform: () => 'linux',
  }
}

function spaced(parts: unknown[]): ReactNode[] {
  const out: ReactNode[] = []
  for (const part of parts) {
    if (part == null || part === false) continue
    if (out.length) out.push(' ')
    out.push(part as ReactNode)
  }
  return out
}

function Row(p: { label?: ReactNode; sub?: ReactNode; value?: ReactNode; onClick?: () => void }) {
  const body = spaced([p.label, p.sub, p.value])
  return p.onClick ? createElement('button', { type: 'button', onClick: p.onClick }, body) : createElement('div', null, body)
}

const ui = {
  Page: (p: { title?: ReactNode; sub?: ReactNode; children?: ReactNode; back?: ReactNode; action?: ReactNode }) =>
    createElement('section', null, spaced([p.back, p.title, p.sub, p.action, p.children])),
  Group: (p: { title?: ReactNode; note?: ReactNode; children?: ReactNode }) => createElement('div', null, spaced([p.title, p.children, p.note])),
  Row,
  Btn: (p: { children?: ReactNode; onClick?: () => void; disabled?: boolean }) =>
    createElement('button', { type: 'button', onClick: p.onClick, disabled: p.disabled }, p.children),
  BackLink: () => null,
  Field: (p: { label?: ReactNode; error?: string | null; children?: ReactNode }) => createElement('label', null, spaced([p.label, p.children, p.error])),
  TextInput: (p: { label: string; value: string; onChange: (v: string) => void }) =>
    createElement('input', { 'aria-label': p.label, value: p.value, onChange: (e: { target: { value: string } }) => p.onChange(String(e.target.value)) }),
  PasswordInput: () => null,
  CodeBoxes: () => null,
  Avatar: ({ name }: { name: string }) => createElement('span', null, name),
  useEscBack() {},
}

function visible(m: Mounted): string {
  const root = m.elements()[0]
  return textOf(root).replace(/\s+/g, ' ').trim()
}

function buttons(m: Mounted): string[] {
  return m.elements().filter((el) => el.type === 'button').map((el) => textOf(el).replace(/\s+/g, ' ').trim())
}

function offersVerify(labels: string[]): boolean {
  return labels.some((label) => /(?:^|\s)Verify(?:\s|$)/.test(label))
}

const paneBindings = {
  ...ui,
  run: runCommand,
  errorText,
  STATE_TONE: { 'This app': 'text-ink', 'Signed in': 'text-ink-2', 'Not signed in': 'text-ink-3' },
}

async function openAccount(backend: Record<string, (args: unknown) => unknown>) {
  const m = mount('v2/sections/settings/account/AccountPane.tsx', 'AccountPane', {
    backend,
    expand: true,
    honestRender: true,
    hookModules: [{
      file: 'v2/sections/settings/account/data.ts',
      name: 'useAccount',
      bindings: { command, initialAccountState, groupSessions: () => [] },
    }],
    bindings: paneBindings,
  })
  mounted.push(m)
  await m.flush()
  return m
}

describe('Account profile actions match the read', () => {
  test('a verified address says Verified and does not offer Verify', async () => {
    const m = await openAccount(reads({ profile: () => profile(EMAIL, true), displayName: 'Sam' }))
    const text = visible(m)
    expect(text).toContain(EMAIL)
    expect(text).toContain('Sam')
    expect(text).toContain('Verified')
    expect(offersVerify(buttons(m))).toBe(false)
    expect(text).not.toContain(TOKEN)
    expect(text).not.toContain('Not verified')
  })

  test('an unverified address offers Verify and does not say Verified', async () => {
    const m = await openAccount(reads({ profile: () => profile(EMAIL, false), displayName: 'Sam' }))
    const text = visible(m)
    expect(text).toContain(EMAIL)
    expect(text).toContain('Sam')
    expect(offersVerify(buttons(m))).toBe(true)
    expect(text).not.toContain('Verified')
    expect(text).not.toContain(TOKEN)
  })

  test('a failed profile read shows the error and Try again, not an empty editable profile', async () => {
    let readsN = 0
    const failed = await openAccount({
      ...reads({ profile: () => { readsN += 1; throw new Error('network') }, displayName: 'Kept Name' }),
    })
    const text = visible(failed)
    expect(text).toContain(ERRORS.network)
    expect(buttons(failed).filter((label) => label === 'Try again')).toEqual(['Try again'])
    expect(text).not.toContain('Kept Name')
    expect(text).not.toContain('Add your name')
    expect(text).not.toContain(EMAIL)
    expect(text).not.toContain(TOKEN)
    expect(offersVerify(buttons(failed))).toBe(false)
    expect(readsN).toBe(1)
    await failed.click('Try again')
    expect(readsN).toBe(2)

    const hidden = await openAccount(reads({ profile: () => { throw new Error(TOKEN) }, displayName: 'Kept Name' }))
    const hiddenText = visible(hidden)
    expect(hiddenText).toContain(errorText(TOKEN))
    expect(hiddenText).not.toContain(TOKEN)
    expect(hiddenText).not.toContain('Kept Name')
    expect(buttons(hidden).filter((label) => label === 'Try again')).toEqual(['Try again'])
  })

  test('saving a name goes through account_set_name and shows the saved name or the plain error', async () => {
    let displayName = 'Sam'
    const saved: string[] = []
    const nameBackend = {
      account_set_name: (args: unknown) => {
        const name = String((args as { name?: string } | undefined)?.name ?? '')
        if (name === 'Nope') throw new Error('rate_limited')
        if (name === TOKEN) throw new Error(TOKEN)
        displayName = name
        return null
      },
    }
    const editor = mount('v2/sections/settings/account/AccountPane.tsx', 'ChangeName', {
      backend: nameBackend,
      expand: true,
      honestRender: true,
      bindings: paneBindings,
      props: { current: 'Sam', onDone: (name: string) => saved.push(name), onCancel() {} },
    })
    mounted.push(editor)

    const field = () => {
      const input = editor.elements().find((el) => el.type === 'input')
      if (!input) throw new Error('no name field')
      return input
    }
    field().props.onChange({ target: { value: '   ' } })
    await editor.flush()
    await editor.click('Save')
    expect(saved).toEqual([])
    expect(visible(editor)).toContain(ERRORS.name_empty)
    expect(editor.calls.filter((call) => call.name === 'account_set_name')).toEqual([])

    field().props.onChange({ target: { value: 'a'.repeat(65) } })
    await editor.flush()
    await editor.click('Save')
    expect(visible(editor)).toContain(ERRORS.name_too_long)
    expect(editor.calls.filter((call) => call.name === 'account_set_name')).toEqual([])

    field().props.onChange({ target: { value: '  Sam Bee  ' } })
    await editor.flush()
    await editor.click('Save')
    await editor.flush()
    expect(editor.calls.filter((call) => call.name === 'account_set_name')).toEqual([{ name: 'account_set_name', args: { name: 'Sam Bee' } }])
    expect(saved).toEqual(['Sam Bee'])
    expect(displayName).toBe('Sam Bee')

    const again = await openAccount(reads({ profile: () => profile(EMAIL, true), displayName }))
    expect(visible(again)).toContain('Sam Bee')
    expect(visible(again)).toContain('Verified')

    const refused = mount('v2/sections/settings/account/AccountPane.tsx', 'ChangeName', {
      backend: nameBackend,
      expand: true,
      honestRender: true,
      bindings: paneBindings,
      props: { current: displayName, onDone: (name: string) => saved.push(name), onCancel() {} },
    })
    mounted.push(refused)
    const refusedField = refused.elements().find((el) => el.type === 'input')!
    refusedField.props.onChange({ target: { value: 'Nope' } })
    await refused.flush()
    await refused.click('Save')
    await refused.flush()
    expect(saved).toEqual(['Sam Bee'])
    expect(displayName).toBe('Sam Bee')
    expect(visible(refused)).toContain(ERRORS.rate_limited)
    expect(visible(refused)).not.toContain('rate_limited')

    refusedField.props.onChange({ target: { value: TOKEN } })
    await refused.flush()
    await refused.click('Save')
    await refused.flush()
    expect(visible(refused)).toContain(errorText(TOKEN))
    expect(visible(refused)).not.toContain(TOKEN)
    expect(saved).toEqual(['Sam Bee'])
  })
})
