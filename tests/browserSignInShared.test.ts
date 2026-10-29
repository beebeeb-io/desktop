/**
 * The browser sign-in is ONE implementation, mounted by two windows (task 1683 slice 2; spec C
 * `docs/specs/2026-10-07-desktop-first-run-sign-in.md` §7.1: “it moves into one shared component that both windows
 * mount”). The state machine is `browserSignIn.ts` (tested in `tests/browserSignIn.test.ts`); each window draws its own
 * markup, because the Windows wizard stays as it was (“Windows and Linux stay unchanged”) and the Mac's cards are the
 * hi-fi's.
 *
 * What this proves: neither window carries a copy of the handoff any more, and the Windows wizard advances exactly as
 * it did when the handoff was inline: only a `fresh` outcome with the keys moves it on, once.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { mount } from './fixtures/componentHarness'
import * as desktopApi from '../src/desktopApi'

const SRC = new URL('../src/', import.meta.url).pathname
function sources(dir = SRC, out = new Map<string, string>()): Map<string, string> {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) sources(path, out)
    else if (/\.(ts|tsx)$/.test(entry)) out.set(relative(SRC, path), readFileSync(path, 'utf8'))
  }
  return out
}

describe('one handoff, two windows', () => {
  const all = sources()

  test('both windows take the state machine from useBrowserSignIn', () => {
    for (const file of ['WindowsFirstRun.tsx', 'macSignIn.tsx']) {
      expect({ file, uses: /import \{[^}]*\buseBrowserSignIn\b[^}]*\} from '\.\/browserSignIn'/.test(all.get(file)!) }).toEqual({ file, uses: true })
    }
  })

  test('only browserSignIn.ts subscribes to the progress event and invokes the handoff', () => {
    const named = (needle: string) => [...all].filter(([, text]) => text.includes(needle)).map(([file]) => file).sort()
    expect(named("'browser-login'")).toEqual(['browserSignIn.ts'])
    // desktopApi.ts names the command in the list of answers that stand when the account changes (a sign-in installs
    // the session), not to call it.
    expect(named("'start_browser_login'")).toEqual(['browserSignIn.ts', 'desktopApi.ts'])
  })

  test('the Windows wizard holds no copy of the handoff: no listener, no done guard', () => {
    const windows = all.get('WindowsFirstRun.tsx')!
    for (const gone of ['listen<', "from '@tauri-apps/api/event'", 'browserDoneRef', 'BrowserLoginEvent', 'setBrowserPhase']) {
      expect({ gone, found: windows.includes(gone) }).toEqual({ gone, found: false })
    }
  })
})

describe('the Windows wizard advances as it always did', () => {
  function openWindowsSignIn() {
    const done: unknown[] = []
    let options: { onSettled: (settled: unknown) => void } | null = null
    const m = mount('WindowsFirstRun.tsx', 'SignInStep', {
      backend: {},
      props: { onDone: (info: unknown) => done.push(info) },
      bindings: {
        ...desktopApi,
        useBrowserSignIn: (given: { onSettled: (settled: unknown) => void }) => {
          options = given
          return { state: { phase: 'idle', userCode: null, verificationUri: null, error: null }, start: async () => {} }
        },
        T: new Proxy({}, { get: () => '' }),
        Btn: () => null,
        Notice: () => null,
        BROWSER_SIGN_IN_INTRO: '',
        BROWSER_DID_NOT_OPEN_HINT: '',
        browserWaitingInstruction: () => '',
        requestAnimationFrame: () => 0,
      },
    })
    return { m, done, settle: (settled: unknown) => options!.onSettled(settled) }
  }

  test('a fresh sign-in whose keys arrived with it (the browser path) moves the wizard on, with the keys unlocked', () => {
    const w = openWindowsSignIn()
    w.settle({ kind: 'fresh', vaultUnlocked: true })
    expect(w.done).toEqual([{ vaultUnlocked: true }])
    w.m.close()
  })

  test('any other outcome moves nothing: a different account is a Mac outcome, and Windows never produces it', () => {
    const w = openWindowsSignIn()
    w.settle({ kind: 'account_mismatch', pendingChanges: 1, unsent: null, unsentError: null })
    w.settle({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false })
    expect(w.done).toEqual([])
    w.m.close()
  })
})

