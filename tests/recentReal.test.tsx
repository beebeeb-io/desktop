// Recent notifications lists the account's alerts. The rows and the new count come from account_notifications
// (noticesFrom / dayOf), an empty list says nothing has arrived, and opening the page does not pretend they were read.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { type Notification, type NotificationList } from '../src/desktopApi'
import { Ctx, type SettingsCtx } from '../src/v2/sections/settings/shell/context'
import * as recent from '../src/v2/sections/settings/today/recent'
import { elementsOf, textOf } from './fixtures/componentHarness'

const settings: SettingsCtx = { pane: 'recent', go: () => {}, dialog: null, open: () => {}, close: () => {} }

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&apos;/g, "'").replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/\s+/g, ' ').trim()

const NOW = new Date(2026, 10, 9, 12, 0, 0)
const TODAY = new Date(2026, 10, 9, 8, 12, 0).toISOString()
const YESTERDAY = new Date(2026, 10, 8, 16, 5, 0).toISOString()

type Listed = { notices: recent.Notice[]; unread: number }
type Ready = { kind: 'ready'; notices: recent.Notice[]; unread: number }
type Failed = { kind: 'failed' }
type RecentState = Ready | Failed | { kind: 'loading' }

function row(over: Partial<Notification> & Pick<Notification, 'id' | 'created_at' | 'title'>): Notification {
  return { type: 'share_received', body: null, read: false, data: null, ...over }
}

function listed(list: NotificationList): Listed {
  const got = (recent.noticesFrom as (list: NotificationList, now?: Date) => Listed)(list, NOW)
  if (!got || !Array.isArray(got.notices) || typeof got.unread !== 'number') {
    throw new Error('noticesFrom must return the rows and the server unread_count')
  }
  return got
}

function draw(state: RecentState, onRetry: () => void = () => {}) {
  const View = (recent as { RecentBody?: (props: { state: RecentState; onRetry: () => void; go: () => void }) => ReactElement }).RecentBody
  if (typeof View !== 'function') throw new Error('RecentBody is not exported from recent.tsx')
  const tree = View({ state, onRetry, go: settings.go })
  const html = renderToStaticMarkup(createElement(Ctx.Provider, { value: settings }, tree))
  return { tree, html, text: text(html) }
}

describe('Recent notifications lists what arrived', () => {
  test('an empty list says nothing yet and the new count is the server unread_count', () => {
    const list: NotificationList = { notifications: [], unread_count: 0 }
    const got = listed(list)
    expect(got.notices).toEqual([])
    expect(got.unread).toBe(list.unread_count)
    expect(recent.dayOf(TODAY, NOW)).toBe('Today')
    const view = draw({ kind: 'ready', notices: got.notices, unread: got.unread })
    expect(view.text).toContain('Nothing yet')
    expect(view.text).toContain('Alerts about your account, such as new sign-ins and files shared with you, show here.')
    expect(view.text).toContain("The new count is the server's. Opening this page does not mark alerts as read.")
    expect(view.html).not.toContain('data-testid="notice"')
    expect(view.text).not.toContain('Today')
    expect(view.text).not.toContain('No notifications')
  })

  test('two alerts are newest first, the new count is unread_count, and a title has no file name', () => {
    const file = 'Quarterly.pdf'
    const list: NotificationList = {
      notifications: [
        row({ id: 'older', type: 'security', title: 'New sign-in from 203.0.113.0', body: 'If this was not you, change your password.', read: true, created_at: YESTERDAY }),
        row({ id: 'newer', type: 'share_received', title: file, body: 'Shared with you.', read: false, data: { path: file }, created_at: TODAY }),
      ],
      unread_count: 3,
    }
    const got = listed(list)
    expect(got.unread).toBe(list.unread_count)
    expect(got.notices.map((n) => n.id)).toEqual(['newer', 'older'])
    expect(got.notices.map((n) => n.day)).toEqual([recent.dayOf(TODAY, NOW), recent.dayOf(YESTERDAY, NOW)])
    expect(got.notices[0]?.day).toBe('Today')
    expect(got.notices[1]?.day).toBe('Yesterday')
    expect(got.notices[0]?.fresh).toBe(true)
    expect(got.notices[1]?.fresh).toBe(false)
    expect(got.notices[0]?.title).not.toContain(file)
    expect(got.notices[0]?.title).toBe('New file shared with you')
    expect(got.notices[1]?.title).toBe('New sign-in from 203.0.113.0')
    expect(got.notices.filter((n) => n.fresh).length).toBe(1)
    const view = draw({ kind: 'ready', notices: got.notices, unread: got.unread })
    expect(view.html.match(/data-testid="notice"/g)).toHaveLength(2)
    expect(view.text).toContain('3 new')
    expect(view.text).not.toContain('1 new')
    expect(view.text).toContain(got.notices[0]!.title)
    expect(view.text).toContain(got.notices[1]!.title)
    expect(view.text).toContain(got.notices[0]!.time)
    expect(view.text).toContain(got.notices[1]!.time)
    expect(view.text.indexOf(got.notices[0]!.title)).toBeLessThan(view.text.indexOf(got.notices[1]!.title))
    expect(view.html).not.toContain(file)
    expect(view.text).toContain("The new count is the server's. Opening this page does not mark alerts as read.")
  })

  test('a failed read says so, and opening the page does not mark alerts read', () => {
    let retried = 0
    const view = draw({ kind: 'failed' }, () => { retried += 1 })
    expect(view.text).toContain("Couldn't load your notifications")
    expect(view.text).toContain('Check your connection and try again.')
    expect(view.html.match(/Try again/g)).toHaveLength(1)
    expect(view.text).not.toContain('Nothing yet')
    expect(view.html).not.toContain('data-testid="notice"')
    const walk = (node: unknown): ReturnType<typeof elementsOf> => elementsOf(node).flatMap((n) => [n, ...walk(n.props.action)])
    const retry = walk(view.tree).filter((n) => n.props.onClick && textOf(n).trim() === 'Try again')
    expect(retry).toHaveLength(1)
    retry[0]!.props.onClick()
    expect(retried).toBe(1)
    const src = readFileSync(new URL('../src/v2/sections/settings/today/recent.tsx', import.meta.url), 'utf8')
    expect(src).toContain("useRead<NotificationList>('account_notifications')")
    expect(src).not.toMatch(/command\(\s*['"]account_mark/)
    expect(src).toContain("The new count is the server's. Opening this page does not mark alerts as read.")
  })
})
