// Notifications › Recent notifications (DW-0791, DW-0791b): the account's recent alerts, newest first, by day.
import { Icon } from '@beebeeb/shared/design'
import { Group, Page, mono } from '../shell/kit'
import { NOTICES } from '../../../source/fixtures/settings-today'
import { BackLink } from './ui'
import { useSettings } from '../shell/context'
import { followHref } from '../shell/links'

export function Recent({ empty }: { empty?: boolean } = {}) {
  const { go } = useSettings()
  const list = empty ? [] : NOTICES
  const days = [...new Set(list.map((n) => n.day))]
  const fresh = list.filter((n) => n.fresh).length
  return (
    <Page title="Recent notifications" sub={empty ? undefined : 'Recent alerts from your account.'} back={<BackLink to="notifications">Notifications</BackLink>}
      action={fresh ? <span className={`rounded-full border border-line px-2 py-0.5 text-[12px] text-ink-2 ${mono}`}>{fresh} new</span> : undefined}>
      {empty ? (
        <Group><div className="px-4 py-8 text-center"><div className="text-[14px] font-medium text-ink">No notifications</div><p className="text-[13px] text-ink-3">Alerts about your account, such as new sign-ins and files shared with you, show here.</p></div></Group>
      ) : days.map((day) => (
        <Group key={day} title={day}>
          {list.filter((n) => n.day === day).map((n) => (
            <a key={n.id} href={n.href} data-testid="notice" aria-label={`${n.fresh ? 'New. ' : ''}${n.title}. ${n.line}. ${n.time}`}
              onClick={(e) => { if (followHref(n.href, go) !== false) e.preventDefault() }}
              className="flex items-start gap-3 px-4 py-2.5 hover:bg-paper-2">
              <Icon name={n.icon} size={16} className="mt-0.5 shrink-0 text-ink-3" />
              <div className="min-w-0 flex-1">
                <div className="text-[13px] leading-5 text-ink">{n.title}{n.fresh ? <span className="ml-2 rounded-full border border-line px-1.5 text-[11px] text-ink-2">new</span> : null}</div>
                <div className="text-[12px] leading-4 text-ink-3">{n.line}</div>
              </div>
              <span className={`shrink-0 text-[12px] text-ink-3 ${mono}`}>{n.time}</span>
            </a>
          ))}
        </Group>
      ))}
    </Page>
  )
}
