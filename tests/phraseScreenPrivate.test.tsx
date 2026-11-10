// The 12 words are typed on three screens. Each one tells Rust it is open, and clears that when it closes.
// Rust must not hide the window from capture: that removes it from the screen the person is typing on.
// The harness runs the real view and the real usePrivateScreen.
import { afterEach, describe, expect, test } from 'bun:test'
import { mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) {
    const m = mounted.pop()!
    m.unmount()
    m.close()
  }
})

const box = () => null
const seen: Array<{ name: string; args: { on: boolean } }> = []

function open(file: string, name: string, props: unknown, bindings: Record<string, unknown>) {
  seen.length = 0
  const m = mount(file, name, {
    backend: {},
    props,
    honestRender: true,
    hookModules: [{
      file: 'v2/auth/privateScreen.ts',
      name: 'usePrivateScreen',
      bindings: {
        hasTauri: () => true,
        command: async (commandName: string, args: { on: boolean }) => {
          seen.push({ name: commandName, args })
          return { ok: true }
        },
      },
    }],
    bindings,
  })
  mounted.push(m)
  return m
}

const words = Array.from({ length: 12 }, () => '')

describe('the recovery phrase tells Rust the screen is open', () => {
  test('setting up this device tells Rust it is open while the 12 words are typed', async () => {
    const m = open('v2/auth/Steps.tsx', 'PhraseView', {
      words, unknown: [], state: 'idle', onWords() {}, onRestore() {}, onApproveInstead() {}, onLost() {},
    }, {
      AuthWindow: box, Heading: box, Note: box, PrimaryButton: box, TextLink: box, SECRET_FIELD: {},
      normalizeWord: (word: string) => word, spreadPaste: () => words,
    })
    await m.flush()
    expect(seen).toEqual([{ name: 'auth_screen_private', args: { on: true } }])
    m.unmount()
    expect(seen.at(-1)).toEqual({ name: 'auth_screen_private', args: { on: false } })
  })

  test('confirming the phrase tells Rust it is open while the words are typed', async () => {
    const m = open('v2/auth/create/views.tsx', 'ConfirmStep', {
      positions: [1], answers: [''], suggestions: [], active: 0, wrong: false, busy: false,
      onAnswer() {}, onActive() {}, onShowAgain() {}, onConfirm() {},
    }, {
      AuthWindow: box, StepLockup: box, Progress: box, Heading: box, Field: box, PrimaryButton: box, Back: box,
      SECRET_FIELD: {}, WORDS_MISMATCH: 'mismatch', noopSubmit: () => () => {},
    })
    await m.flush()
    expect(seen).toEqual([{ name: 'auth_screen_private', args: { on: true } }])
    m.unmount()
    expect(seen.at(-1)).toEqual({ name: 'auth_screen_private', args: { on: false } })
  })

  test('recovering with the phrase tells Rust it is open while the words are typed', async () => {
    const m = open('v2/auth/forgot/index.tsx', 'RecoverPhraseView', {
      email: 'a@b.c', words, unknown: [], state: 'idle', onEmail() {}, onWords() {}, onVerify() {}, onLost() {},
    }, {
      AuthWindow: box, StepLockup: box, Heading: box, Note: box, Field: box, PrimaryButton: box, TextLink: box,
      SECRET_FIELD: {}, phraseFilled: () => true, recoverErrorText: () => '', normalizeWord: (word: string) => word,
      spreadPaste: () => words,
    })
    await m.flush()
    expect(seen).toEqual([{ name: 'auth_screen_private', args: { on: true } }])
    m.unmount()
    expect(seen.at(-1)).toEqual({ name: 'auth_screen_private', args: { on: false } })
  })
})
