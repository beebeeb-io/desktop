/**
 * Slice 34: the conflict window resolves the two copies it shows.
 *
 * The window renders both sides of a real conflict (the preview the daemon returns), names each side
 * (the URL's fileName — the same file on both devices) and its real size from the conflict's own
 * data, and says in plain words when a side's bytes are not on this PC. A choice goes through the
 * real `resolve_conflict` command and the window closes only after that command confirms; a refusal
 * leaves it open with the error. Keep both says where the copy went without inventing a filename:
 * the out-of-scope resolver names the copy its own way and `resolve_conflict` returns no name
 * (nightctl ask-002421-17685), so the window names no file at all.
 *
 * The wire contract for a side is `ConflictContentSide` (src-tauri/src/engine_bridge/types.rs):
 * `size_bytes`, `text`, `unavailable_reason` — it carries no name and no time. The fixture below
 * injects exactly those fields and no others.
 *
 * The component harness executes the real `ConflictWindow` declaration from src/ConflictWindow.tsx
 * against a scripted backend (tests/fixtures/componentHarness.ts).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { formatBytes } from '../src/desktopApi'
import { diffLines } from '../src/diffLines'
import { PARKED_REPLACE_QUESTION, parkedResolutionNotice } from '../src/parkedConflict'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

interface Side {
  size_bytes?: number | null
  text?: string | null
  unavailable_reason?: string | null
}

const preview = (over: { is_text?: boolean; local?: Side; remote?: Side; version?: number | null } = {}) => ({
  is_text: over.is_text ?? false,
  local: over.local ?? { size_bytes: 1_258_291 },
  remote: over.remote ?? { size_bytes: 3_363_149 },
  version: over.version ?? null,
})

function deferred<T = void>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

const SEARCH = '?window=conflict&fileId=f1&fileName=Contract%20draft.pdf'

function open(opts: {
  value?: ReturnType<typeof preview>
  resolve?: (args: { fileId: string; choice: string }) => unknown
  search?: string
}) {
  const calls: Array<{ name: string; args: any }> = []
  const toasts: any[] = []
  const win = { closed: 0, search: opts.search ?? SEARCH }
  const windowBinding = {
    location: { search: win.search },
    close() { win.closed++ },
  }
  const invoke = async (name: string, args: any) => {
    calls.push({ name, args })
    if (name === 'resolve_conflict') return opts.resolve ? opts.resolve(args) : undefined
    throw new Error(`unscripted command ${name}`)
  }
  const m = mount('ConflictWindow.tsx', 'ConflictWindow', {
    backend: {},
    bindings: {
      window: windowBinding,
      useToast: () => ({ showToast: (t: any) => toasts.push(t) }),
      invoke,
      conflictContentPreview: () => Promise.resolve(opts.value ? { ok: true, value: opts.value } : { ok: false, reason: 'no preview' }),
      parkedConflictPreview: () => Promise.resolve({ ok: false, reason: 'no preview' }),
      resolveParkedConflict: () => Promise.resolve({ ok: false, reason: 'no preview' }),
      formatBytes,
      diffLines,
      PARKED_REPLACE_QUESTION,
      parkedResolutionNotice,
      DiffLine: () => null,
      DiffPane: () => null,
      setTimeout: (fn: () => void) => { queueMicrotask(fn); return 0 },
    },
  })
  mounted.push(m)
  return { m, calls, toasts, win }
}

const shown = (m: Mounted) => textOf(m.tree()).replace(/\s+/g, ' ').trim()
const buttons = (m: Mounted) =>
  m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).replace(/\s+/g, ' ').trim())

describe('ConflictWindow with a real conflict', () => {
  test('each side shows the file name and its real size, no invented time, and a side not on this PC says so and offers the other', async () => {
    const { m } = open({
      value: preview({
        local: { size_bytes: 1_258_291 },
        remote: {
          size_bytes: 3_363_149,
          unavailable_reason: 'These bytes are not on this PC yet.',
        },
      }),
    })
    await m.flush()
    const text = shown(m)
    // The name is the URL's fileName: the same file, shown on both sides, plus the heading.
    expect(text.match(/Contract draft\.pdf/g)?.length ?? 0).toBe(3)
    expect(text).toContain('1 MB')
    expect(text).toContain('3 MB')
    // The wire contract carries no time, so none is invented.
    expect(text).not.toContain('Today')
    expect(text).toContain('These bytes are not on this PC yet.')
    expect(buttons(m)).toEqual(expect.arrayContaining(['Keep Mine', 'Keep Theirs', 'Keep Both', 'Keep the other version']))
  })

  test('a choice calls resolve_conflict and closes only after it confirms; a refusal keeps the window open with the error', async () => {
    const pending = deferred()
    const ok = open({
      value: preview({ remote: { size_bytes: 3_363_149, unavailable_reason: 'These bytes are not on this PC yet.' } }),
      resolve: () => pending.promise,
    })
    await ok.m.flush()
    await ok.m.clickNoWait('Keep the other version')
    await ok.m.flush()
    expect(ok.calls).toContainEqual({ name: 'resolve_conflict', args: { fileId: 'f1', choice: 'local' } })
    expect(ok.win.closed).toBe(0)
    expect(shown(ok.m)).not.toContain('Conflict resolved')

    pending.resolve()
    await ok.m.flush()
    expect(ok.win.closed).toBe(1)
    expect(shown(ok.m)).toContain('Conflict resolved')

    const refused = open({
      value: preview(),
      resolve: () => Promise.reject(new Error('The other version is not on this PC.')),
    })
    await refused.m.flush()
    await refused.m.click('Keep Mine')
    expect(refused.win.closed).toBe(0)
    expect(shown(refused.m)).toContain('The other version is not on this PC.')
    expect(buttons(refused.m)).toEqual(expect.arrayContaining(['Keep Mine', 'Keep Theirs', 'Keep Both']))
  })

  test('Keep both leaves a copy beside the original and names no file the resolver does not name', async () => {
    const keepBoth = open({ value: preview() })
    await keepBoth.m.flush()
    await keepBoth.m.click('Keep Both')
    expect(keepBoth.calls).toContainEqual({ name: 'resolve_conflict', args: { fileId: 'f1', choice: 'both' } })
    const text = shown(keepBoth.m)
    // resolve_conflict returns no name and the resolver uses its own scheme, so the window must not
    // promise a specific filename; it says where the copy went instead.
    expect(text).toContain('beside the original')
    expect(text).toContain('folder')
    expect(text).not.toContain('-conflicted copy')
    expect(text).not.toContain('(conflict -')
    expect(keepBoth.win.closed).toBe(1)
  })
})
