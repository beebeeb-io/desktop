/**
 * Desktop v2 B05e (2440 item 1): a stray click cannot undo a comparison.
 * While a seal (They match / They don't match / Share anyway) is in flight, every way out of the
 * comparison dialogs is inert — Esc, backdrop click and every footer button — and a second click
 * runs no second command. When it settles the dialog moves exactly once.
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { elementsOf, expand, loadComponent, mount, textOf } from './fixtures/componentHarness'

const member = { id: 'm-jonas', name: 'Jonas Meier', email: 'jonas@northwind.studio', access: 'Can edit' as const }
const groups = ['1234 5678', '9012 3456']

/** A Modal stand-in that records whether a way out (Esc / backdrop) is wired. */
const Modal = (p: any) => createElement('div', { role: 'dialog', 'data-onclose': p.onClose ? 'yes' : 'no' }, p.title, p.children, p.footer)
const Groups = (p: any) => createElement('div', null, p.groups.join(' '))
const Icon = () => null

const buttonsOf = (tree: any) => elementsOf(tree).filter((el) => el.type === 'button')
const modalOf = (tree: any) => elementsOf(tree).find((el) => el.props.role === 'dialog')

describe('a comparison dialog while a seal runs', () => {
  const SafetyNumberDialog = loadComponent('v2/sections/shared/Dialogs.tsx', 'SafetyNumberDialog', {
    React: { createElement, Fragment }, Modal, Groups, BTN: '',
  })

  test('safety_number_dialog_is_inert_while_busy', () => {
    const busy = expand(SafetyNumberDialog({ member, groups, busy: true, onMatch() {}, onNoMatch() {}, onClose() {} }))
    expect(modalOf(busy)?.props['data-onclose']).toBe('no')
    expect(buttonsOf(busy).every((b) => b.props.disabled === true)).toBe(true)

    const idle = expand(SafetyNumberDialog({ member, groups, busy: false, onMatch() {}, onNoMatch() {}, onClose() {} }))
    expect(modalOf(idle)?.props['data-onclose']).toBe('yes')
    expect(buttonsOf(idle).every((b) => !b.props.disabled)).toBe(true)
  })

  const MismatchDialog = loadComponent('v2/sections/shared/Dialogs.tsx', 'MismatchDialog', {
    React: { createElement, Fragment }, Modal, Groups, Icon, BTN: '', BTN_AMBER: '', RED_TEXT: '',
  })

  test('mismatch_dialog_is_inert_while_busy', () => {
    const busy = expand(MismatchDialog({ member, groups, busy: true, onRemove() {}, onCompare() {}, onClose() {} }))
    expect(modalOf(busy)?.props['data-onclose']).toBe('no')
    expect(buttonsOf(busy).every((b) => b.props.disabled === true)).toBe(true)

    const idle = expand(MismatchDialog({ member, groups, busy: false, onRemove() {}, onCompare() {}, onClose() {} }))
    expect(modalOf(idle)?.props['data-onclose']).toBe('yes')
    expect(buttonsOf(idle).every((b) => !b.props.disabled)).toBe(true)
  })

  const KeyChangedAlert = loadComponent('v2/sections/shared/Dialogs.tsx', 'KeyChangedAlert', {
    React: { createElement, Fragment }, Modal, BTN: '',
  })

  test('key_changed_alert_is_inert_while_busy', () => {
    const busy = expand(KeyChangedAlert({ member, busy: true, onCompare() {}, onShareAnyway() {}, onCancel() {} }))
    expect(modalOf(busy)?.props['data-onclose']).toBe('no')
    expect(buttonsOf(busy).every((b) => b.props.disabled === true)).toBe(true)

    const idle = expand(KeyChangedAlert({ member, busy: false, onCompare() {}, onShareAnyway() {}, onCancel() {} }))
    expect(modalOf(idle)?.props['data-onclose']).toBe('yes')
    expect(buttonsOf(idle).every((b) => !b.props.disabled)).toBe(true)
  })
})

describe('a double click on a seal runs one command', () => {
  const EMPTY_LISTS = { invitations: [], links: [], requests: [], pending: [] }
  const EMPTY_SHARES = { withYou: [], removed: [], byYou: [], folder: {}, name: {}, items: {} }

  test('they_match_twice_on_a_refused_invite_marks_and_invites_once', async () => {
    const calls: string[] = []
    const toasts: any[] = []
    const source = { kind: 'desktop', shared: async () => [], subscribe: () => () => {} }
    const api = {
      shareList: async () => ({ ok: true, value: EMPTY_LISTS }),
      sharePeople: async () => ({ ok: true, value: [member] }),
      shareSafetyNumber: async () => ({ ok: true, value: { groups } }),
      shareMarkCompared: async () => { calls.push('mark'); return { ok: true } },
      shareInvite: async () => { calls.push('invite'); return { ok: true, value: { status: 'shared' } } },
      shareRevoke: async () => ({ ok: true }),
      shareAcceptChangedKey: async () => ({ ok: true }),
      shareSetAccess: async () => ({ ok: true }),
      shareLeave: async () => ({ ok: true }),
      shareLinkUpdate: async () => ({ ok: true }),
      shareLinkCopy: async () => ({ ok: true, value: { display: 'link' } }),
      shareAccept: async () => ({ ok: true }),
      shareDecline: async () => ({ ok: true }),
      fileRequestCreate: async () => ({ ok: true, value: { display: 'link' } }),
      fileRequestCopy: async () => ({ ok: true, value: { display: 'link' } }),
      fileRequestUpdate: async () => ({ ok: true }),
      fileRequestRemove: async () => ({ ok: true }),
      failureText: () => 'failed',
      openFailureText: () => 'failed',
    }
    const SafetyNumberDialog = (p: any) => createElement('div', { role: 'dialog' },
      createElement('button', { onClick: p.onMatch }, 'They match'),
      createElement('button', { onClick: p.onNoMatch }, 'They don’t match'),
      createElement('button', { onClick: p.onClose }, 'Close'))
    const PersonDialog = () => createElement('div', { role: 'dialog' }, 'person')
    const PeopleDialog = () => createElement('div', { role: 'dialog' }, 'people')
    const Null = () => null
    const refused = { kind: 'safety', itemId: 'item-1', member, groups, refused: { email: member.email, canEdit: true } }

    const m = mount('v2/sections/shared/index.tsx', 'Section', {
      props: { nav: { route: { section: 'shared', id: null, sub: null }, go() {}, back() {}, forward() {} } },
      backend: {},
      expand: true,
      bindings: {
        useDriveSource: () => source,
        showToast: (t: any) => toasts.push(t),
        BannerSlot: Null,
        BTN: '', BTN_AMBER: '',
        Menu: Null,
        fmtWhen: () => '',
        DetailsDialog: Null, VersionsDialog: Null,
        InviteDialog: Null, KeyChangedAlert: Null, LeaveAlert: Null, LinkPasswordDialog: Null,
        MismatchDialog: Null, NewRequestDialog: Null, PeopleDialog, PersonDialog, RemoveAccessAlert: Null,
        SafetyNumberDialog,
        afterMarkCompared: (x: any) => x,
        byMeMenu: () => [], EXPIRY_CHOICES: [], linkExpiry: () => '', requestLine: () => ({ files: '', state: null, open: true }),
        requestMenu: () => [], withMeMenu: () => [],
        api,
        seed: (key: string, fallback: any) => key === 'shared.dialog' ? refused : key === 'shared.members' ? [member] : fallback,
        EMPTY_LISTS, EMPTY_SHARES,
        days: () => null, bytes: () => null, peopleNames: () => [], classifyShares: () => EMPTY_SHARES,
        Heading: Null, EmptyAccount: Null,
        toast: (r: any, done?: string) => toasts.push({ r, done }),
      },
    })
    await m.flush()
    expect(m.elements().filter((el) => el.props.role === 'dialog')).toHaveLength(1)

    const match = m.elements().find((el) => el.type === 'button' && textOf(el.props.children) === 'They match')!
    const first = match.props.onClick()
    const second = match.props.onClick()
    await Promise.all([first, second])
    await m.flush()

    expect(calls.filter((c) => c === 'mark')).toHaveLength(1)
    expect(calls.filter((c) => c === 'invite')).toHaveLength(1)
    expect(toasts.filter((t) => t?.r && t.r.ok === false)).toHaveLength(0)
    expect(m.elements().filter((el) => el.props.role === 'dialog')).toHaveLength(1)
    m.close()
  })
})
