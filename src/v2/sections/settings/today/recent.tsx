// Notifications › Recent notifications (DW-0791, DW-0791b): the account's recent alerts, newest first, by day. The
// list is the server's (`account_notifications`, P1c), never the board's example alerts. No mark-read command is
// registered, so opening this page leaves the server's unread count as it was.
import { type ReactElement } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { type NotificationList } from '../../../../desktopApi'
import { Btn, Group, Page, mono } from '../shell/kit'
import { useRead } from '../shell/data'
import { BackLink, ErrorBanner, Skeleton } from './ui'
import { useSettings, type SettingsCtx } from '../shell/context'
import { followHref } from '../shell/links'

export interface Notice { id: string; day: string; icon: 'users' | 'share' | 'mail' | 'shield' | 'bell'; title: string; line: string; time: string; fresh?: boolean; href?: string }

/** Where an alert leads, by the server's type: a share opens Shared with me, a request its list, security the activity. */
function placeOf(type: string): Pick<Notice, 'icon' | 'href'> {
  if (type.startsWith('share')) return { icon: 'share', href: '?section=shared&sub=with-me' }
  if (type.startsWith('file_request')) return { icon: 'mail', href: '?section=shared&sub=requests' }
  if (type === 'security' || type === 'new_device_login') return { icon: 'shield', href: '?section=settings&page=signin-activity' }
  return { icon: 'bell' }
}

/** "Today", "Yesterday", or "3 Nov", in the PC's own time. */
export function dayOf(iso: string, now = new Date()): string {
  const d = new Date(iso)
  const days = Math.round((new Date(now.toDateString()).getTime() - new Date(d.toDateString()).getTime()) / 86_400_000)
  if (days === 0) return 'Today'
  if (days === 1) return 'Yesterday'
  return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

/** A path, or a word with a lettered extension (report.pdf). A dotted address (203.0.113.0) is not a file name. */
function fileNamesIn(text: string): string[] {
  const found: string[] = []
  const re = /(?:^|\s)((?:[^\s/\\]+[\\/])*[^\s/\\]+\.[A-Za-z0-9]{1,8})(?=$|[\s,.;])/g
  for (const match of text.matchAll(re)) {
    const token = match[1] ?? ''
    const ext = token.slice(token.lastIndexOf('.') + 1)
    if (/[A-Za-z]/.test(ext)) found.push(token)
  }
  for (const raw of text.match(/(?:^|\s)((?:[^\s/\\]+[\\/])+\S*)/g) ?? []) {
    const token = raw.trim().replace(/[,.;]+$/, '')
    if (token && !found.includes(token)) found.push(token)
  }
  return found
}

/** What the row is, when the server's title is missing or is only a file name. The words are the server's own titles. */
function labelFor(type: string): string {
  if (type === 'share_claimed') return 'Share claim pending'
  if (type === 'share_approved') return 'Share approved'
  if (type.startsWith('share')) return 'New file shared with you'
  if (type.startsWith('file_request')) return 'File request'
  if (type === 'security' || type === 'new_device_login') return 'New sign-in'
  if (type === 'welcome') return 'Welcome to Beebeeb'
  if (type === 'storage_warning') return 'Storage warning'
  if (type === 'file_updated') return 'A file was updated'
  const words = type.replace(/[._]+/g, ' ').trim()
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : 'Alert'
}

/** The alert's title. The server's words, with a file name left out (alerts never name a file). */
function titleOf(type: string, title: string): string {
  const names = fileNamesIn(title)
  if (names.length === 0) return title.trim() || labelFor(type)
  let stripped = title
  for (const name of names) stripped = stripped.replaceAll(name, ' ')
  stripped = stripped.replace(/\s+/g, ' ').replace(/\s+([,.;])/g, '$1').trim().replace(/^[,.;]+|[,.;]+$/g, '')
  if (stripped && fileNamesIn(stripped).length === 0) return stripped
  return labelFor(type)
}

/** The server's alerts as the page lists them: newest first. The new count is `unread_count`, not how many rows are unread. */
export function noticesFrom(list: NotificationList, now = new Date()): { notices: Notice[]; unread: number } {
  const notices = [...list.notifications]
    .filter((n) => !Number.isNaN(new Date(n.created_at).getTime()))
    .sort((a, b) => b.created_at.localeCompare(a.created_at))
    .map((n) => ({
      id: n.id, title: titleOf(n.type ?? '', n.title), line: n.body ?? '', fresh: !n.read,
      day: dayOf(n.created_at, now), time: new Date(n.created_at).toTimeString().slice(0, 5), ...placeOf(n.type ?? ''),
    }))
  return { notices, unread: list.unread_count }
}

export type RecentState =
  | { kind: 'loading' }
  | { kind: 'failed' }
  | { kind: 'ready'; notices: Notice[]; unread: number }

const SERVER_COUNT = "The new count is the server's. Opening this page does not mark alerts as read."

/** The page for one read: loading, the refusal, or the server's rows. It never marks an alert read. */
export function RecentBody({ state, onRetry, go }: { state: RecentState; onRetry: () => void; go: SettingsCtx['go'] }): ReactElement {
  const back = <BackLink to="notifications">Notifications</BackLink>
  if (state.kind === 'loading') {
    return <Page title="Recent notifications" back={back}>{[0, 1].map((i) => <Skeleton key={i} className="h-[88px] w-full" />)}</Page>
  }
  if (state.kind === 'failed') {
    return (
      <Page title="Recent notifications" back={back}>
        <ErrorBanner title="Couldn't load your notifications" action={<Btn onClick={onRetry}>Try again</Btn>}>Check your connection and try again.</ErrorBanner>
      </Page>
    )
  }
  const { notices, unread } = state
  const days = [...new Set(notices.map((n) => n.day))]
  return (
    <Page title="Recent notifications" sub={notices.length ? 'Recent alerts from your account.' : undefined} back={back}
      action={unread > 0 ? <span className={`rounded-full border border-line px-2 py-0.5 text-[12px] text-ink-2 ${mono}`}>{unread.toLocaleString('en')} new</span> : undefined}>
      {!notices.length ? (
        <Group><div className="px-4 py-8 text-center"><div className="text-[14px] font-medium text-ink">Nothing yet</div><p className="text-[13px] text-ink-3">Alerts about your account, such as new sign-ins and files shared with you, show here.</p></div></Group>
      ) : days.map((day) => (
        <Group key={day} title={day}>
          {notices.filter((n) => n.day === day).map((n) => {
            const body = (
              <>
                <Icon name={n.icon} size={16} className="mt-0.5 shrink-0 text-ink-3" />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] leading-5 text-ink">{n.title}{n.fresh ? <span className="ml-2 rounded-full border border-line px-1.5 text-[11px] text-ink-2">new</span> : null}</div>
                  {n.line ? <div className="text-[12px] leading-4 text-ink-3">{n.line}</div> : null}
                </div>
                <span className={`shrink-0 text-[12px] text-ink-3 ${mono}`}>{n.time}</span>
              </>
            )
            const label = `${n.fresh ? 'New. ' : ''}${n.title}. ${n.line ? `${n.line}. ` : ''}${n.time}`
            return n.href ? (
              <a key={n.id} href={n.href} data-testid="notice" aria-label={label}
                onClick={(e) => { if (followHref(n.href!, go) !== false) e.preventDefault() }}
                className="flex items-start gap-3 px-4 py-2.5 hover:bg-paper-2">{body}</a>
            ) : <div key={n.id} data-testid="notice" aria-label={label} className="flex items-start gap-3 px-4 py-2.5">{body}</div>
          })}
        </Group>
      ))}
      <p className="px-1 text-[12px] leading-4 text-ink-3">{SERVER_COUNT}</p>
    </Page>
  )
}

export function Recent({ empty }: { empty?: boolean } = {}) {
  const { go } = useSettings()
  const read = useRead<NotificationList>('account_notifications')
  if (empty) return <RecentBody state={{ kind: 'ready', notices: [], unread: 0 }} onRetry={() => {}} go={go} />
  if (read.state === 'loading') return <RecentBody state={{ kind: 'loading' }} onRetry={read.reload} go={go} />
  if (read.state === 'failed') return <RecentBody state={{ kind: 'failed' }} onRetry={read.reload} go={go} />
  const { notices, unread } = noticesFrom(read.value)
  return <RecentBody state={{ kind: 'ready', notices, unread }} onRetry={read.reload} go={go} />
}
