// The window says whether the account read answered: the sidebar label while pending, the email when it lands, and
// the existing network sentence with Try again when the profile read never answers. No fixture account.
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, type ReactNode } from 'react'
import { command } from '../src/desktopApi'
import { ERRORS, PROFILE_TIMEOUT_MS, accountRowLabel, errorText, initialAccountState } from '../src/v2/sections/settings/account/data'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

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

const paneBindings = {
  ...ui,
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

describe('The window says if you are signed in', () => {
  test('while the profile read has not answered the sidebar label is Account', () => {
    expect(accountRowLabel(null, null, true)).toBe('Account')
    // Even a name or email already in hand does not show while the read is still pending.
    expect(accountRowLabel('Sam', 'sam@example.eu', true)).toBe('Account')
  })

  test('a profile with an email and no display name shows that email', () => {
    expect(accountRowLabel(null, 'sam@example.eu', false)).toBe('sam@example.eu')
    expect(accountRowLabel('', 'sam@example.eu', false)).toBe('sam@example.eu')
    // A display name wins over the email; a failed read with nothing known leaves the row empty.
    expect(accountRowLabel('Sam', 'sam@example.eu', false)).toBe('Sam')
    expect(accountRowLabel(null, null, false)).toBe('')
  })

  test('a profile read that never answers shows the network sentence and Try again, not a blank profile', async () => {
    expect(PROFILE_TIMEOUT_MS).toBe(8000)
    const never = new Promise<never>(() => {})
    const m = await openAccount({
      account_profile: () => never,
      account_display_name: () => ({ display_name: null }),
      account_session_list: () => ({ sessions: [] }),
      account_security_score: () => ({ factors: [] }),
      account_activity: () => ({ events: [] }),
      account_activity_logging: () => ({ on: true }),
      account_passkeys_list: () => [],
      sync_status: () => ({ logged_in: true }),
      app_version: () => ({ version: '0.9.4', release: true }),
      desktop_platform: () => 'linux',
    })
    const text = visible(m)
    expect(text).toContain(ERRORS.network)
    expect(buttons(m).filter((label) => label === 'Try again')).toEqual(['Try again'])
    // The email is still empty, so the pane does not draw an editable profile.
    expect(text).not.toContain('Add your name')
    expect(text).not.toContain('@')
  })
})
