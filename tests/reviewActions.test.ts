/**
 * What a Review row's button does (task 1683 slice 4, the Home add-on): one place for Review and for Home, so the
 * same press means the same thing in both. Moved out of `ReviewPane`; tests/reviewPane.test.tsx still runs it there.
 * What this proves: each action's one effect, and that a failure is one toast. What it does NOT prove: the sheets
 * ("Decide…" only hands the entry to whoever opens the sheet).
 */
import { describe, expect, test } from 'bun:test'
import { BILLING_URL, type VersionConflictEntry } from '../src/desktopApi'
import { makeReviewActions } from '../src/reviewActions'

const entry = { id: 'e', file_id: 'f', file_name: 'a.md', kind: 'conflict', status: 's', detail: 'd', action: 'open_conflict' } as VersionConflictEntry

function setup(over: Record<string, unknown> = {}) {
  const log = { dismissed: [] as string[], changed: 0, toasts: [] as unknown[], decided: [] as unknown[], urls: [] as string[], reauth: 0 }
  const act = makeReviewActions({
    showToast: (toast: unknown) => { log.toasts.push(toast); return '' },
    dismiss: async (id: string) => { log.dismissed.push(id); return { ok: true, value: undefined } },
    onChanged: () => { log.changed += 1 },
    onDecide: (e: unknown) => { log.decided.push(e) },
    openUrl: async (url: string) => { log.urls.push(url); return { ok: true, value: undefined } },
    forceReauth: async () => { log.reauth += 1; return { ok: true, value: { warning: null } } },
    ...over,
  } as never)
  return { act, log }
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('makeReviewActions', () => {
  test('Dismiss refreshes only after the host confirms it', async () => {
    const { act, log } = setup()
    await act({ ...entry, id: 'kept:42' }, { kind: 'dismiss', label: 'Dismiss' })
    expect(log.dismissed).toEqual(['kept:42'])
    expect(log.changed).toBe(1)
    expect(log.reauth).toBe(0)
    const failed = setup({ dismiss: async () => ({ ok: false, reason: 'Still open', unsupported: false }) })
    await failed.act(entry, { kind: 'dismiss', label: 'Dismiss' })
    expect(failed.log.changed).toBe(0)
    expect(failed.log.toasts).toEqual([{ variant: 'error', title: 'Couldn’t dismiss this entry', message: 'Still open' }])
  })

  test('a failed browser action reports once', async () => {
    const { act, log } = setup({ openUrl: async () => ({ ok: false, reason: 'Browser unavailable', unsupported: false }) })
    await act(entry, { kind: 'upgrade', label: 'Add storage' })
    expect(log.toasts).toEqual([{ variant: 'error', title: 'Couldn’t open your browser', message: 'Browser unavailable' }])
  })

  test('"Decide…" hands the entry to the one who opens the sheet, and does nothing else', async () => {
    const { act, log } = setup()
    act(entry, { kind: 'open_conflict', label: 'Decide…' })
    await settle()
    expect(log.decided).toEqual([entry])
    expect(log.urls).toEqual([])
    expect(log.reauth).toBe(0)
  })

  test('"Add storage" opens the billing page in the browser', async () => {
    const { act, log } = setup()
    act(entry, { kind: 'upgrade', label: 'Add storage' })
    await settle()
    expect(log.urls).toEqual([BILLING_URL])
    expect(log.decided).toEqual([])
  })

  test('"Manage plan" opens the billing page too: adding storage does not cure a lapsed trial (review of #142)', async () => {
    const { act, log } = setup()
    act(entry, { kind: 'manage_plan', label: 'Manage plan' })
    await settle()
    expect(log.urls).toEqual([BILLING_URL])
    expect(log.decided).toEqual([])
    expect(log.reauth).toBe(0)
  })

  test('"Sign in again" starts the in-place sign-in and says nothing when it works', async () => {
    const { act, log } = setup()
    act(entry, { kind: 'sign_in_again', label: 'Sign in again' })
    await settle()
    expect(log.reauth).toBe(1)
    expect(log.toasts).toEqual([])
  })

  test('a sign-in that cannot start is one error toast; a warning with it is shown as info', async () => {
    const failing = setup({ forceReauth: async () => ({ ok: false, reason: 'no window', unsupported: false }) })
    failing.act(entry, { kind: 'sign_in_again', label: 'Sign in again' })
    await settle()
    expect(failing.log.toasts).toEqual([{ variant: 'error', title: 'Couldn’t open sign-in', message: 'no window' }])
    const warned = setup({ forceReauth: async () => ({ ok: true, value: { warning: { sentence: 'We couldn’t confirm the old session ended.' } } }) })
    warned.act(entry, { kind: 'sign_in_again', label: 'Sign in again' })
    await settle()
    expect(warned.log.toasts).toEqual([{ variant: 'info', message: 'We couldn’t confirm the old session ended.' }])
  })
})
