// Security factors show the read, and nothing the read did not say. A control for a state the command did not
// report (backup codes before two-factor is on, Remove for a passkey that is not in the list, a switch when the
// activity read failed) is not drawn. A refused action uses the plain ERRORS line and leaves the read in place.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { ERRORS, initialAccountState, type AccountState } from '../src/v2/sections/settings/account/data'
import { Activity, Passkeys, TotpStatus } from '../src/v2/sections/settings/account/Factors'
import { ago, day } from '../src/v2/sections/settings/account/ui'
import type { ActivityEvent, Passkey } from '../src/v2/sections/settings/account/types'
import { allElements, click, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()
const markup = (node: ReactElement) => {
  const html = renderToStaticMarkup(node)
  return { html, text: textOf(html) }
}

function account(partial: {
  totp?: Partial<AccountState['totp']>
  passkeys?: Passkey[]
  activity?: ActivityEvent[]
  activityLogging?: boolean
} = {}): AccountState {
  const base = initialAccountState()
  return {
    ...base,
    activityLogging: partial.activityLogging ?? base.activityLogging,
    passkeys: partial.passkeys ?? base.passkeys,
    activity: partial.activity ?? base.activity,
    totp: { ...base.totp, ...partial.totp },
  }
}

const noop = () => {}
const added = '2026-03-03T12:00:00Z'
const used = '2026-05-01T12:00:00Z'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  // React only treats an input's `change` as onChange when `oninput` exists on document, and only
  // when `input.type` is a text type. The mini DOM has neither until these are set, before react-dom loads.
  Object.defineProperty(dom.document, 'oninput', { value: null, configurable: true })
  Object.defineProperty(MiniElement.prototype, 'type', {
    configurable: true,
    get() { return this.getAttribute('type') ?? (this.tagName === 'INPUT' ? 'text' : '') },
    set(value: string) { this.setAttribute('type', String(value)) },
  })
  const proto = MiniElement.prototype as MiniElement & {
    querySelector: (selector: string) => MiniElement | null
  }
  proto.querySelector = function querySelector(selector: string) {
    const parts = selector.split(',').map((part) => part.trim())
    const found = allElements(this).slice(1)
    return found.find((el) => parts.some((part) => matches(el, part))) ?? null
  }
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

function matches(el: MiniElement, selector: string): boolean {
  if (selector.startsWith('[') && selector.endsWith(']')) {
    const body = selector.slice(1, -1)
    const eq = body.indexOf('=')
    if (eq < 0) return el.hasAttribute(body)
    return el.getAttribute(body.slice(0, eq)) === body.slice(eq + 1).replace(/['"]/g, '')
  }
  return el.tagName === selector.toUpperCase()
}

function buttons(root: MiniNode, label: string): MiniElement[] {
  return allElements(root).filter((el) => el.tagName === 'BUTTON' && el.textContent.trim() === label)
}

function send(container: MiniNode, target: MiniElement, type: string) {
  const event = {
    type,
    target,
    srcElement: target,
    currentTarget: container,
    bubbles: true,
    cancelable: true,
    defaultPrevented: false,
    isTrusted: true,
    button: 0,
    timeStamp: Date.now(),
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor(type, true)) fn(event)
  for (const fn of container.listenersFor(type, false)) fn(event)
}

/** Set the input the way the value tracker notices: through the prototype setter, then the input event. */
function typeInto(container: MiniNode, input: MiniElement, value: string) {
  const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')
  descriptor?.set?.call(input, value)
  send(container, input, 'input')
  send(container, input, 'change')
}

function scriptInvoke(fail: (name: string) => Error | null) {
  const calls: { name: string; args: unknown }[] = []
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (name: string, args: unknown) => {
      calls.push({ name, args })
      const error = fail(name)
      if (error) throw error
      return null
    },
  }
  return calls
}

async function mount(node: ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
  return {
    container,
    async settle() {
      for (let i = 0; i < 6; i++) await React.act(async () => { await Promise.resolve() })
    },
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

describe('Security factors show their real state', () => {
  test('two-factor shows on, off, or a failed read, and no backup-code action the read does not allow', () => {
    const on = markup(<TotpStatus
      state={account({ totp: { on: true, addedAt: added, lastUsedAt: used, codesLeft: 3, codesTotal: 8 } })}
      onBack={noop} onNewCodes={noop} onMove={noop} onTurnedOff={noop}
    />)
    expect(on.text).toContain('On')
    expect(on.text).toContain(`Added ${day(added)}`)
    expect(on.text).toContain(`Last used ${ago(used)}`)
    expect(on.text).toContain('3 of 8')
    expect(on.text).toContain('Make new backup codes')
    expect(on.text).toContain('Move to a new authenticator')
    expect(on.text).toContain('Turn off two-factor')

    const unreported = markup(<TotpStatus
      state={account({ totp: { on: true, addedAt: null, lastUsedAt: null, codesLeft: null, codesTotal: 8 } })}
      onBack={noop} onNewCodes={noop} onMove={noop} onTurnedOff={noop}
    />)
    expect(unreported.text).toContain('Added Not reported')
    expect(unreported.text).toContain('Last used Not reported')
    expect(unreported.text).toContain('Not reported')
    expect(unreported.text).not.toContain('of 8')
    expect(unreported.text).not.toContain('not yet')
    expect(unreported.text).toContain('Make new backup codes')

    const off = markup(<TotpStatus
      state={account({ totp: { on: false, addedAt: null, lastUsedAt: null, codesLeft: null, codesTotal: 8 } })}
      onBack={noop} onNewCodes={noop} onMove={noop} onTurnedOff={noop}
    />)
    expect(off.text).toContain('Off')
    expect(off.text).not.toContain('Two-factor on')
    expect(off.text).not.toContain('Make new backup codes')
    expect(off.text).not.toContain('Move to a new authenticator')
    expect(off.text).not.toContain('Turn off two-factor')
    expect(off.text).not.toContain('Backup codes')
    expect(off.html.match(/Try again/g) ?? []).toHaveLength(0)

    let retried = 0
    const failed = markup(<TotpStatus
      state={account({ totp: { on: true, addedAt: added, lastUsedAt: used, codesLeft: 3, codesTotal: 8 } })}
      read={{ ok: false, code: 'network' }}
      onRetry={() => { retried += 1 }}
      onBack={noop} onNewCodes={noop} onMove={noop} onTurnedOff={noop}
    />)
    expect(failed.text).toContain(ERRORS.network)
    expect(failed.text).not.toContain('On')
    expect(failed.text).not.toContain('Off')
    expect(failed.text).not.toContain('Make new backup codes')
    expect(failed.text).not.toContain('Move to a new authenticator')
    expect(failed.text).not.toContain('Turn off two-factor')
    expect(failed.text).not.toContain(day(added))
    expect(failed.html.match(/Try again/g)).toHaveLength(1)
    expect(retried).toBe(0)
  })

  test('passkeys show the listed names and no Remove for a passkey the read did not list', () => {
    const desk: Passkey = { id: 'desk', name: 'Desk key', created_at: added, last_used_at: null, prf: null as unknown as boolean }
    const phone: Passkey = { id: 'phone', name: 'Phone key', created_at: '2026-04-04T12:00:00Z', last_used_at: used, prf: true }
    const listed = markup(<Passkeys
      state={account()}
      read={{ ok: true, value: [desk, phone] }}
      onBack={noop} setState={noop} menuOn="desk"
    />)
    expect(listed.text).toContain('2 passkeys')
    expect(listed.text).toContain('Desk key')
    expect(listed.text).toContain('Phone key')
    expect(listed.text).toContain(`Added ${day(added)}`)
    expect(listed.text).toContain(`Added ${day(phone.created_at)}`)
    expect(listed.text).toContain('Last used Not reported')
    expect(listed.text).toContain(`Last used ${ago(used)}`)
    expect(listed.text).toContain('Not reported')
    expect(listed.text).toContain('Signs in and unlocks your data')
    expect(listed.text).not.toContain('Signs in only')
    expect(listed.text).not.toContain('Rename')
    expect(buttonsFrom(listed.html)).toContain('Remove')

    const empty = markup(<Passkeys
      state={account({ passkeys: [desk] })}
      read={{ ok: true, value: [] }}
      onBack={noop} setState={noop} removing="Desk key" menuOn="desk"
    />)
    expect(empty.text).toContain('0 passkeys')
    expect(empty.text).toContain('No passkeys yet')
    expect(empty.text).not.toContain('Desk key')
    expect(empty.text).not.toContain('Remove')
    expect(empty.text).toContain('Add a passkey')

    const failed = markup(<Passkeys
      state={account({ passkeys: [desk] })}
      read={{ ok: false, code: 'network' }}
      onBack={noop} setState={noop} removing="Desk key" menuOn="desk"
    />)
    expect(failed.text).toContain(ERRORS.network)
    expect(failed.text).not.toContain('Desk key')
    expect(failed.text).not.toContain('Remove')
    expect(failed.text).not.toContain('Add a passkey')
    expect(failed.text).not.toContain('No passkeys yet')
    expect(failed.text).not.toContain('Rename')
    expect(failed.html.match(/Try again/g)).toHaveLength(1)
  })

  test('activity is newest first with the country, and a failed read offers Try again', async () => {
    const now = Date.parse('2026-11-09T12:00:00Z')
    const events: ActivityEvent[] = [
      { id: 'old', what: 'Signed in on Linux', at: '2026-01-02T10:00:00Z', country: 'DE', address: null },
      { id: 'mid', what: 'Changed a password', at: '2026-08-02T10:00:00Z', country: 'FI', address: null },
      { id: 'new', what: 'New sign-in', at: '2026-11-09T08:00:00Z', country: null, address: null },
    ]
    const on = markup(<Activity
      state={account({ activityLogging: true, activity: events })}
      onBack={noop} setState={noop} onPassword={noop} onDevices={noop} now={now}
    />)
    expect(on.html.indexOf('New sign-in')).toBeGreaterThan(-1)
    expect(on.html.indexOf('New sign-in')).toBeLessThan(on.html.indexOf('Changed a password'))
    expect(on.html.indexOf('Changed a password')).toBeLessThan(on.html.indexOf('Signed in on Linux'))
    expect(on.text).toContain('DE')
    expect(on.text).toContain('FI')
    expect(on.text).toContain('Not reported')
    expect(on.html).toContain('aria-checked="true"')

    const off = markup(<Activity
      state={account({ activityLogging: false, activity: events })}
      onBack={noop} setState={noop} onPassword={noop} onDevices={noop} now={now}
    />)
    expect(off.text).toContain('Security logging is off')
    expect(off.html).toContain('aria-checked="false"')
    expect(off.text).not.toContain('Signed in on Linux')
    expect(off.text).not.toContain('New sign-in')

    let retried = 0
    const failed = markup(<Activity
      state={account({ activityLogging: true, activity: events })}
      read={{ ok: false, code: 'network' }}
      onRetry={() => { retried += 1 }}
      onBack={noop} setState={noop} onPassword={noop} onDevices={noop} now={now}
    />)
    expect(failed.text).toContain(ERRORS.network)
    expect(failed.text).not.toContain('New sign-in')
    expect(failed.text).not.toContain('Signed in on Linux')
    expect(failed.text).not.toContain('Record sign-in activity')
    expect(failed.html).not.toContain('aria-checked')
    expect(failed.html.match(/Try again/g)).toHaveLength(1)

    const view = await mount(<Activity
      state={account({ activityLogging: true, activity: events })}
      read={{ ok: false, code: 'network' }}
      onRetry={() => { retried += 1 }}
      onBack={noop} setState={noop} onPassword={noop} onDevices={noop} now={now}
    />)
    const again = buttons(view.container, 'Try again')
    expect(again).toHaveLength(1)
    await React.act(async () => { click(view.container, again[0]!) })
    expect(retried).toBe(1)
    await view.unmount()
  })

  test('a refused action shows the plain error and leaves the factor as it was read', async () => {
    let turnedOff = 0
    const totpCalls = scriptInvoke((name) => (name === 'account_totp_disable' ? new Error('invalid_code') : new Error(`unexpected ${name}`)))
    const totp = await mount(<TotpStatus
      state={account({ totp: { on: true, addedAt: added, lastUsedAt: used, codesLeft: 3, codesTotal: 8 } })}
      onBack={noop} onNewCodes={noop} onMove={noop} onTurnedOff={() => { turnedOff += 1 }}
    />)
    const turnOff = buttons(totp.container, 'Turn off two-factor')
    expect(turnOff).toHaveLength(1)
    await React.act(async () => { click(totp.container, turnOff[0]!) })
    const code = allElements(totp.container).find((el) => el.tagName === 'INPUT' && el.getAttribute('aria-label') === '6-digit code')
    expect(code).toBeTruthy()
    await React.act(async () => { typeInto(totp.container, code!, '482913') })
    const confirm = buttons(totp.container, 'Turn off').filter((el) => !el.hasAttribute('disabled'))
    expect(confirm).toHaveLength(1)
    await React.act(async () => { click(totp.container, confirm[0]!) })
    await totp.settle()
    expect(totpCalls.map((call) => call.name)).toEqual(['account_totp_disable'])
    expect(totp.container.textContent).toContain(ERRORS.invalid_code)
    expect(totp.container.textContent).not.toContain('Incorrect code. Try again.')
    expect(turnedOff).toBe(0)
    expect(totp.container.textContent).toContain('On')
    await totp.unmount()

    let writes = 0
    const passkey: Passkey = { id: 'desk', name: 'Desk key', created_at: added, last_used_at: null, prf: true }
    const removeCalls = scriptInvoke((name) => (name === 'account_passkeys_remove' ? new Error('wrong_password') : new Error(`unexpected ${name}`)))
    const passkeys = await mount(<Passkeys
      state={account({ passkeys: [passkey] })}
      onBack={noop} setState={() => { writes += 1 }} menuOn="desk"
    />)
    const remove = buttons(passkeys.container, 'Remove')
    expect(remove).toHaveLength(1)
    await React.act(async () => { click(passkeys.container, remove[0]!) })
    const password = allElements(passkeys.container).find((el) => el.tagName === 'INPUT' && el.getAttribute('aria-label') === 'Password')
    expect(password).toBeTruthy()
    await React.act(async () => { typeInto(passkeys.container, password!, 'correct horse') })
    const confirmRemove = buttons(passkeys.container, 'Remove').filter((el) => !el.hasAttribute('disabled'))
    expect(confirmRemove).toHaveLength(1)
    await React.act(async () => { click(passkeys.container, confirmRemove[0]!) })
    await passkeys.settle()
    expect(removeCalls.map((call) => call.name)).toEqual(['account_passkeys_remove'])
    expect(passkeys.container.textContent).toContain(ERRORS.wrong_password)
    expect(passkeys.container.textContent).toContain('Desk key')
    expect(writes).toBe(0)
    await passkeys.unmount()

    let loggingWrites = 0
    const loggingCalls = scriptInvoke((name) => (name === 'account_set_activity_logging' ? new Error('network') : new Error(`unexpected ${name}`)))
    const activity = await mount(<Activity
      state={account({ activityLogging: false, activity: [{ id: 'old', what: 'Signed in on Linux', at: '2026-01-02T10:00:00Z', country: 'DE', address: null }] })}
      onBack={noop} setState={() => { loggingWrites += 1 }} onPassword={noop} onDevices={noop} now={Date.parse('2026-11-09T12:00:00Z')}
    />)
    const toggle = allElements(activity.container).filter((el) => el.tagName === 'BUTTON' && el.getAttribute('aria-label') === 'Record sign-in activity')
    expect(toggle).toHaveLength(1)
    expect(toggle[0]!.getAttribute('aria-checked')).toBe('false')
    await React.act(async () => { click(activity.container, toggle[0]!) })
    await activity.settle()
    expect(loggingCalls.map((call) => call.name)).toEqual(['account_set_activity_logging'])
    expect(activity.container.textContent).toContain(ERRORS.network)
    expect(activity.container.textContent).not.toContain('Signed in on Linux')
    const after = allElements(activity.container).find((el) => el.getAttribute('aria-label') === 'Record sign-in activity')
    expect(after?.getAttribute('aria-checked')).toBe('false')
    expect(loggingWrites).toBe(0)
    await activity.unmount()
  })
})

/** Button labels in static markup, in document order. */
function buttonsFrom(html: string): string[] {
  return [...html.matchAll(/<button\b[^>]*>(.*?)<\/button>/gs)].map((match) => match[1].replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim())
}
