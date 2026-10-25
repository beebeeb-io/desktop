/**
 * Must-render rows 12 and 13, lead ruling FB-24, review M4: a sign-out or a Lock that HAPPENED but could
 * not confirm one step is `Ok({warning: {code, sentence}})`, not an `Err`. `warning` is always present
 * (null when every step was confirmed). The codes are a closed set; an `Err` still means "it did not
 * happen". So:
 * - `clearSession` and `lockVault` (src/desktopApi.ts) return the parsed `SessionActionOutcome`;
 * - a warning's sentence is shown as a neutral status line, never under a "Couldn’t …" title;
 * - every caller of either command reads `warning` (the source contract at the end).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { accountSessionRevision } from '../src/accountSession'
import { clearSession, command, loadSyncStatus, lockVault, parseSessionActionOutcome, SESSION_ACTION_WARNING_CODES } from '../src/desktopApi'
import { rustSource, rustStr } from './fixtures/rustConstants'

const SENTENCE = {
  finder_removal_unconfirmed: rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING'),
  finder_lock_unconfirmed: rustStr('lib.rs', 'FINDER_LOCK_UNCONFIRMED_WARNING'),
  engine_stop_unconfirmed: rustStr('lib.rs', 'LOCK_ENGINE_UNCONFIRMED_WARNING'),
}

describe('the SessionActionOutcome contract', () => {
  test('the closed set of codes is exactly what Rust\'s ActionWarning::code() returns', () => {
    const lib = rustSource('lib.rs')
    const impl = lib.slice(lib.indexOf('impl ActionWarning {'), lib.indexOf('impl serde::Serialize for ActionWarning'))
    const body = impl.slice(impl.indexOf('fn code(self)'), impl.indexOf('fn sentence(self)'))
    const rust = [...body.matchAll(/=> "([a-z_]+)"/g)].map((m) => m[1]).sort()
    expect(rust).toHaveLength(3)
    expect([...SESSION_ACTION_WARNING_CODES].sort()).toEqual(rust)
  })

  test('both commands answer it', () => {
    const lib = rustSource('lib.rs')
    // Task 1882 (rebase onto main): the command also takes the app handle, to raise the kept-folder alert; rustfmt
    // puts its arguments on their own lines, so compare without whitespace. Task 1887: and the calling window (a result
    // that needs showing waits for that window's acknowledgement) and `keep_unsent`, which `clearSession`'s
    // `keepUnsent` fills.
    const squeezed = lib.replace(/\s+/g, '')
    expect(squeezed).toContain(
      'asyncfnclear_session(app:tauri::AppHandle,window:tauri::WebviewWindow,state:State<\'_,AppState>,forget_email:Option<bool>,keep_unsent:Option<KeepUnsentArg>,)->Result<SessionActionOutcome,String>{',
    )
    expect(lib).toContain('async fn lock_vault(state: State<\'_, AppState>) -> Result<SessionActionOutcome, String> {')
  })
})

describe('parseSessionActionOutcome', () => {
  test('no warning, and each code with its sentence', () => {
    expect(parseSessionActionOutcome({ warning: null })).toEqual({ warning: null })
    for (const code of SESSION_ACTION_WARNING_CODES) {
      expect(parseSessionActionOutcome({ warning: { code, sentence: SENTENCE[code] } })).toEqual({ warning: { code, sentence: SENTENCE[code] } })
    }
  })

  test('anything outside the contract is rejected (null), never guessed', () => {
    for (const wrong of [
      null,
      undefined,
      {},
      { warning: undefined },
      { warning: { code: 'something_new', sentence: 'x' } },
      { warning: { code: 'finder_removal_unconfirmed' } },
      { warning: { code: 'finder_removal_unconfirmed', sentence: '  ' } },
      { warning: { code: 'finder_removal_unconfirmed', sentence: 3 } },
      { warning: 'finder_removal_unconfirmed' },
      [],
      'ok',
    ]) {
      expect(parseSessionActionOutcome(wrong)).toBeNull()
    }
  })
})

describe('clearSession and lockVault', () => {
  const previous = (globalThis as any).window
  const originalWarn = console.warn
  afterEach(() => { (globalThis as any).window = previous; console.warn = originalWarn })
  const backend = (answers: Record<string, (args: any) => unknown>) => {
    const calls: Array<{ name: string; args: any }> = []
    ;(globalThis as any).window = {
      __TAURI_INTERNALS__: {
        invoke: async (name: string, args: any) => {
          calls.push({ name, args })
          const answer = answers[name]
          if (!answer) throw new Error(`unscripted ${name}`)
          return answer(args)
        },
      },
    }
    return calls
  }

  test('a sign-out that happened with a warning is Ok with that warning', async () => {
    const warning = { code: 'finder_removal_unconfirmed', sentence: SENTENCE.finder_removal_unconfirmed }
    backend({ clear_session: () => ({ warning }) })
    expect(await clearSession()).toEqual({ ok: true, value: { warning } })
  })

  test('forgetEmail is passed only when asked for', async () => {
    const calls = backend({ clear_session: () => ({ warning: null }) })
    await clearSession()
    await clearSession({ forgetEmail: true })
    // Tauri's invoke sends `{}` when no arguments are given; Rust reads a missing forgetEmail as false.
    expect(calls.map((c) => c.args ?? {})).toEqual([{}, { forgetEmail: true }])
  })

  test('a Lock that happened with a warning is Ok with that warning', async () => {
    const warning = { code: 'engine_stop_unconfirmed', sentence: SENTENCE.engine_stop_unconfirmed }
    backend({ lock_vault: () => ({ warning }) })
    expect(await lockVault()).toEqual({ ok: true, value: { warning } })
  })

  test('an Err is still "it did not happen"', async () => {
    backend({ lock_vault: () => { throw 'Could not stop the sync engine' } })
    expect(await lockVault()).toEqual({ ok: false, reason: 'Could not stop the sync engine', unsupported: false })
  })

  test('an Ok whose outcome cannot be read is still Ok (it happened), with no warning and a trace of codes only', async () => {
    const traced: unknown[][] = []
    console.warn = (...args: unknown[]) => { traced.push(args) }
    backend({ clear_session: () => ({ warning: { code: 'something_new', sentence: 'a path /Users/sam' } }) })
    expect(await clearSession()).toEqual({ ok: true, value: { warning: null } })
    expect(traced).toEqual([['clear_session', 'outcome_unreadable']])
  })
})

/**
 * `command()` answers "Account changed" when this WebView observed a new session revision while a command ran.
 * A sign-out moves that revision itself, and on a Mac a status read it held back behind the engine slot is
 * released already carrying the new revision. When that read is observed before the sign-out's own answer, the
 * sign-out still happened: its answer stands. Every other command still says "Account changed".
 */
describe('the answer of a command that ends the session itself stands when the revision moved while it ran', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })

  /** `name` ends the session; before it answers, this WebView observes a status read that carries the new revision. */
  async function revisionMovesWhileRunning(name: string, answer: unknown, start: number) {
    const native = { loggedIn: true, revision: start }
    const seen = { atAnswer: null as number | null }
    ;(globalThis as any).window = {
      __TAURI_INTERNALS__: {
        invoke: async (called: string) => {
          if (called === 'sync_status') return { logged_in: native.loggedIn, session_revision: native.revision }
          if (called !== name) throw new Error(`unscripted ${called}`)
          native.loggedIn = false
          native.revision += 2
          await loadSyncStatus()
          seen.atAnswer = accountSessionRevision()
          return answer
        },
      },
    }
    await loadSyncStatus()
    expect(accountSessionRevision()).toBe(start)
    return { native, seen }
  }

  test('a sign-out: Ok with its warning', async () => {
    const warning = { code: 'finder_removal_unconfirmed', sentence: SENTENCE.finder_removal_unconfirmed }
    const { native, seen } = await revisionMovesWhileRunning('clear_session', { warning }, 700)
    const result = await clearSession()
    expect(seen.atAnswer).toBe(native.revision)
    expect(result).toEqual({ ok: true, value: { warning } })
  })

  test('the account switch’s sign-out (forgetEmail): Ok with its warning', async () => {
    const warning = { code: 'finder_removal_unconfirmed', sentence: SENTENCE.finder_removal_unconfirmed }
    const { native, seen } = await revisionMovesWhileRunning('clear_session', { warning }, 710)
    const result = await clearSession({ forgetEmail: true })
    expect(seen.atAnswer).toBe(native.revision)
    expect(result).toEqual({ ok: true, value: { warning } })
  })

  test('a Lock that left no Keychain session (Rust moves the revision then): Ok with its warning', async () => {
    const warning = { code: 'engine_stop_unconfirmed', sentence: SENTENCE.engine_stop_unconfirmed }
    const { native, seen } = await revisionMovesWhileRunning('lock_vault', { warning }, 720)
    const result = await lockVault()
    expect(seen.atAnswer).toBe(native.revision)
    expect(result).toEqual({ ok: true, value: { warning } })
  })

  // Task 1683 slice 2: the sign-in runs in the Beebeeb window, which `AccountSessionBoundary` polls every second, so a
  // status read can observe the revision a sign-in moves before the sign-in's own answer arrives. The sign-in INSTALLS
  // the session, so the revision moving is the sign-in having worked: its answer (a LoginOutcome with no account data
  // of the new session in it) stands, as a sign-out's does. The Keychain and recovery-phrase unlocks do not move the
  // revision (the session is already present), so they keep the guard.
  test('a sign-in: Ok with its outcome, for each way in (password, second factor, browser)', async () => {
    let start = 760
    const outcome = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null }
    for (const name of ['desktop_login', 'desktop_login_2fa', 'start_browser_login']) {
      const { native, seen } = await revisionMovesWhileRunning(name, outcome, start)
      const result = await command<unknown>(name)
      expect(seen.atAnswer).toBe(native.revision)
      expect({ name, result }).toEqual({ name, result: { ok: true, value: outcome } })
      start += 10
    }
  })

  // PR #146 review, P3: the hint's answer is a flag of this Mac, not of an account. A sign-in or a lock that moves the revision
  // while the read is out must not turn it into "Account changed" (which hides the hint) or lose a "Got it".
  test('the one-time hint\'s two commands carry no account data: their answers stand', async () => {
    let start = 780
    for (const [name, answer] of [['get_menu_bar_hint_seen', false], ['mark_menu_bar_hint_seen', null]] as const) {
      const { native, seen } = await revisionMovesWhileRunning(name, answer, start)
      const result = await command<unknown>(name)
      expect(seen.atAnswer).toBe(native.revision)
      expect({ name, result }).toEqual({ name, result: { ok: true, value: answer } })
      start += 10
    }
  })

  test('any other command still says "Account changed"', async () => {
    let start = 730
    for (const name of ['account_email', 'unlock_vault', 'popover_snapshot', 'desktop_unlock_with_recovery_phrase', 'open_onboarding_window']) {
      const { native, seen } = await revisionMovesWhileRunning(name, 'sam@example.eu', start)
      const result = await command<unknown>(name)
      expect(seen.atAnswer).toBe(native.revision)
      expect({ name, result }).toEqual({ name, result: { ok: false, reason: 'Account changed. Please try again.', unsupported: false } })
      start += 10
    }
  })
})

/**
 * Every caller of either command reads `warning`. The commands are reached only through the two
 * wrappers, and every function that calls a wrapper reads `.warning` of what it got back.
 */
describe('source contract: every caller reads the warning', () => {
  const root = new URL('../src/', import.meta.url).pathname
  const files = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name)
      return statSync(path).isDirectory() ? files(path) : /\.tsx?$/.test(name) ? [path] : []
    })
  const sources = files(root).map((path) => ({ file: path.slice(root.length), text: readFileSync(path, 'utf8') }))

  test('the raw commands are invoked only by their wrappers in desktopApi.ts', () => {
    // A surface may use the command name as its own busy key, never to invoke it.
    const invoking = sources
      .filter(({ text }) => /(command|invoke|runAction|run)(<[^>]*>)?\(\s*['"](clear_session|lock_vault)['"]\s*\)/.test(text) || /(command|invoke)(<[^>]*>)?\(\s*['"](clear_session|lock_vault)['"]\s*,/.test(text))
      .map(({ file }) => file)
    expect(invoking).toEqual([])
    const api = sources.find(({ file }) => file === 'desktopApi.ts')!.text
    expect(api).toContain("return sessionAction('clear_session', ")
    expect(api).toContain("return sessionAction('lock_vault')")
  })

  test('every function that calls clearSession or lockVault reads .warning', () => {
    const offenders: string[] = []
    let sites = 0
    for (const { file, text } of sources) {
      if (file === 'desktopApi.ts') {
        // forceReauth is the one caller inside desktopApi.
        const body = text.slice(text.indexOf('export async function forceReauth'))
        expect(body).toContain('.warning')
        sites += 1
        continue
      }
      if (file === 'signOutFlow.ts') {
        // The Settings sign-out flow (task 1887): its API object only passes the call through, and the flow's own
        // `signOut` is the one caller, a class method this function census cannot see; it reads the answer's warning.
        const start = text.indexOf('private async signOut(')
        expect(start, 'the flow signs out in signOut').toBeGreaterThan(-1)
        const body = text.slice(start, text.indexOf('\n  }\n', start))
        expect(body).toContain('this.api.clearSession(')
        expect(body).toContain('result.value.warning')
        sites += 1
        continue
      }
      // Every reference outside an import: a call, or the function handed to a runner (MacSettings).
      const code = text.replace(/^import[\s\S]*?from '[^']+'\n/gm, (imports) => ' '.repeat(imports.length))
      for (const match of code.matchAll(/\b(clearSession|lockVault)\b/g)) {
        sites += 1
        const before = text.slice(0, match.index)
        const start = Math.max(before.lastIndexOf('\nfunction '), before.lastIndexOf('\nexport function '), before.lastIndexOf('\nexport default function '))
        const next = text.indexOf('\nfunction ', match.index)
        const body = text.slice(start, next < 0 ? undefined : next)
        if (!body.includes('.warning')) offenders.push(`${file}: ${match[1]}`)
      }
    }
    expect(offenders).toEqual([])
    // forceReauth, AccountSwitchStep (Linux's onboarding window), SwitchScreen (the same step in the Beebeeb window, task
    // 1683 slice 2: it reads `.warning`, so the census still finds no caller that drops it), MacSettings (lock), the
    // Settings sign-out flow, pages/Account (lock, sign-out), AccountView.
    expect(sites).toBe(8)
  })
})
