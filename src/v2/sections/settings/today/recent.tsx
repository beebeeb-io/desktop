// Notifications › Recent notifications (DW-0791, DW-0791b): the account's recent alerts, newest first, by day. The
// list is the server's (`account_notifications`, P1c), never the board's example alerts.
import { Icon } from '@beebeeb/shared/design'
import { type Notification, type NotificationList } from '../../../../desktopApi'
import { Btn, Group, Page, mono } from '../shell/kit'
import { useRead } from '../shell/data'
import { BackLink, ErrorBanner, Skeleton } from './ui'
import { useSettings } from '../shell/context'
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

/** The server's alerts as the page lists them: newest first, unread ones marked new. */
export function noticesFrom(list: Notification[], now = new Date()): Notice[] {
  return [...list]
    .filter((n) => !Number.isNaN(new Date(n.created_at).getTime()))
    .sort((a, b) => b.created_at.localeCompare(a.created_at))
    .map((n) => ({
      id: n.id, title: n.title, line: n.body ?? '', fresh: !n.read,
      day: dayOf(n.created_at, now), time: new Date(n.created_at).toTimeString().slice(0, 5), ...placeOf(n.type ?? ''),
    }))
}

export function Recent({ empty }: { empty?: boolean } = {}) {
  const { go } = useSettings()
  const read = useRead<NotificationList>('account_notifications')
  const back = <BackLink to="notifications">Notifications</BackLink>
  if (!empty && read.state === 'loading') {
    return <Page title="Recent notifications" back={back}>{[0, 1].map((i) => <Skeleton key={i} className="h-[88px] w-full" />)}</Page>
  }
  if (!empty && read.state === 'failed') {
    return (
      <Page title="Recent notifications" back={back}>
        <ErrorBanner title="Couldn't load your notifications" action={<Btn onClick={read.reload}>Try again</Btn>}>Check your connection and try again.</ErrorBanner>
      </Page>
    )
  }
  const list = empty || read.state !== 'ready' ? [] : noticesFrom(read.value.notifications)
  const days = [...new Set(list.map((n) => n.day))]
  const fresh = list.filter((n) => n.fresh).length
  return (
    <Page title="Recent notifications" sub={list.length ? 'Recent alerts from your account.' : undefined} back={back}
      action={fresh ? <span className={`rounded-full border border-line px-2 py-0.5 text-[12px] text-ink-2 ${mono}`}>{fresh} new</span> : undefined}>
      {!list.length ? (
        <Group><div className="px-4 py-8 text-center"><div className="text-[14px] font-medium text-ink">No notifications</div><p className="text-[13px] text-ink-3">Alerts about your account, such as new sign-ins and files shared with you, show here.</p></div></Group>
      ) : days.map((day) => (
        <Group key={day} title={day}>
          {list.filter((n) => n.day === day).map((n) => {
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
    </Page>
  )
}
